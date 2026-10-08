//! Image tools: crop with handles, the resize, save-a-copy, and batch
//! sheets, and the background removal result. Long work runs on a worker
//! as a `Task`; its result comes back to the window as `Done`.
use super::{
    actions::{self, file_size, start_export},
    app::{file_name, invalidate, with_state, State},
    disk,
    document::{Phase, PointerEvent, PointerKind},
    files,
    render::Painter,
    sheet::{self, Change, Control},
    theme::Rgba,
    widgets::Rect,
    worker::{Edits, Event, Job, Request, SaveKind},
};
use crate::model::{BatchJob, BatchResize, BatchResult, ExportOptions, Frame, ImageEdit, ImageFormat};
use std::{
    collections::HashMap,
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc, OnceLock},
    time::{Duration, Instant},
};
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::HWND,
        UI::{
            Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, VIRTUAL_KEY, VK_BACK, VK_DELETE, VK_ESCAPE, VK_RETURN},
            Shell::ShellExecuteW,
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    },
};

pub(super) const CROP_HINT: &str = "Drag over the image to choose the area to keep. Escape cancels.";
const SELECT_HINT: &str = "Drag to select an area. Ctrl+K crops to it, and Delete clears it. Escape cancels.";
const ADJUST_HINT: &str = "Drag the edges or corners to adjust. Press Enter to crop or Escape to cancel.";
/// Typing pauses this long before a size estimate starts.
const ESTIMATE_DELAY: Duration = Duration::from_millis(300);
/// The error `imaging::batch` gives files it skipped after a cancel.
const CANCELED: &str = "Canceled.";

#[derive(Default)]
pub(super) struct Tools {
    pub(super) crop: Option<CropBox>,
    /// The selection tool uses the crop box: Some(true) for an ellipse,
    /// Some(false) for a rectangle, None for the crop tool.
    pub(super) select: Option<bool>,
    /// Files from the Explorer Convert or Resize verb; true for Resize.
    pub(super) verb: Option<(bool, Vec<PathBuf>)>,
    estimate: Estimate,
    cutout: Option<Cutout>,
    summary: Option<BatchSummary>,
    refresh: bool,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct CropBox {
    /// Left, top, right, bottom, from 0 to 1 across the displayed image.
    rect: [f32; 4],
    grab: Option<Grab>,
}

/// A drag in progress: which edges follow the pointer, from where.
#[derive(Clone, Copy, Debug)]
struct Grab {
    edges: [bool; 4],
    from: [f32; 2],
    start: [f32; 4],
}

struct Cutout {
    path: PathBuf,
    frame: Arc<Frame>,
    seconds: f32,
}

pub(super) struct BatchSummary {
    pub(super) folder: PathBuf,
    pub(super) saved: usize,
    /// "name: reason" for each file that failed.
    pub(super) failed: Vec<String>,
    pub(super) skipped: usize,
}

impl BatchSummary {
    fn of(folder: PathBuf, results: Vec<BatchResult>) -> Self {
        let mut summary = Self { folder, saved: 0, failed: Vec::new(), skipped: 0 };
        for result in results {
            match result.output {
                Ok(_) => summary.saved += 1,
                Err(error) if error == CANCELED => summary.skipped += 1,
                Err(error) => summary.failed.push(format!("{}: {error}", file_name(&result.input))),
            }
        }
        summary
    }
}

/// Work for a worker thread. Only plain data crosses the thread.
pub(super) enum Task {
    Estimate { id: u64, extra: Vec<ImageEdit>, options: ExportOptions },
    Export { output: PathBuf, options: ExportOptions },
    Batch { files: Vec<PathBuf>, folder: PathBuf, job: BatchJob, cancel: Arc<AtomicBool> },
    Cutout,
    SaveFrame { frame: Arc<Frame>, output: PathBuf },
    ClipboardFrame { frame: Arc<Frame> },
}

pub(super) enum Done {
    Estimated(u64, Result<u64, String>),
    Progress(usize, usize),
    Batched(BatchSummary),
    Cutout(PathBuf, Result<(Arc<Frame>, f32), String>),
    /// CF_DIBV5 and PNG bytes, ready for the clipboard.
    Clipboard(Result<(Vec<u8>, Vec<u8>), String>),
}

impl Task {
    pub(super) fn run(self, request: &Request, progress: &(dyn Fn(usize, usize) + Sync)) -> Event {
        let source = request.source();
        let edits = request.sessions.get(&request.path).cloned().unwrap_or_default();
        match self {
            Task::Estimate { id, extra, options } => {
                let mut all = edits.image;
                all.extend(extra);
                Event::Tool(Done::Estimated(id, crate::imaging::estimate_size(source, &all, &options)))
            }
            Task::Export { output, options } => {
                let result = crate::imaging::export_with(source, &output, &edits.image, &options)
                    .map(|_| ())
                    .map_err(|e| format!("Could not save {}: {e}", output.display()));
                Event::Saved(request.path.clone(), edits, true, result)
            }
            Task::Batch { files, folder, job, cancel } => {
                // ponytail: the batch holds the task worker, so hover OCR waits until it ends.
                // Give batches their own thread if that wait matters.
                let results = crate::imaging::batch(&files, &folder, &job, progress, &cancel);
                Event::Tool(Done::Batched(BatchSummary::of(folder, results)))
            }
            Task::Cutout => {
                let started = Instant::now();
                let result = crate::background::cutout(source, &edits.image)
                    .map(|frame| (Arc::new(frame), started.elapsed().as_secs_f32()));
                Event::Tool(Done::Cutout(request.path.clone(), result))
            }
            Task::SaveFrame { frame, output } => {
                let result = crate::imaging::export_frame(&frame, &output)
                    .map(|_| format!("Saved {} with a transparent background.", file_name(&output)));
                Event::Background(output, result)
            }
            Task::ClipboardFrame { frame } => Event::Tool(Done::Clipboard(crate::imaging::encode_clipboard(&frame))),
        }
    }
}

/// Handles a worker result on the window thread. Sheets wait for `tick`.
pub(super) fn received(hwnd: HWND, s: &mut State, done: Done) {
    match done {
        Done::Estimated(id, result) => {
            if id == s.tools.estimate.id && s.tools.estimate.request.is_some() {
                s.tools.estimate.result = Some(result);
                s.tools.refresh = true;
            }
        }
        Done::Progress(done, total) => s.status = format!("Converting {done} of {total}. Press Escape to cancel."),
        Done::Batched(summary) => {
            s.exporting = false;
            s.cancel = None;
            s.status = format!("Conversion finished: {} saved, {} failed.", summary.saved, summary.failed.len());
            s.tools.summary = Some(summary);
        }
        Done::Cutout(path, result) => {
            s.exporting = false;
            match result {
                Ok((frame, seconds)) => {
                    s.status = "Background removed.".into();
                    s.tools.cutout = Some(Cutout { path, frame, seconds });
                }
                Err(error) => s.status = error,
            }
        }
        Done::Clipboard(result) => {
            s.status = match result.and_then(|(dib, png)| crate::imaging::put_clipboard(hwnd, &dib, &png)) {
                Ok(()) => "Copied the image with a transparent background.".into(),
                Err(error) => error,
            };
        }
    }
}

/// Runs on every window tick: starts a due size estimate, then shows a
/// finished result once no sheet is open.
pub(super) unsafe fn tick(hwnd: HWND) {
    enum Next {
        Refresh,
        Cutout(Cutout),
        Summary(BatchSummary),
        Verb(bool, Vec<PathBuf>),
    }
    let next = with_state(|s| {
        send_estimate(s);
        if s.sheet.is_some() {
            return std::mem::take(&mut s.tools.refresh).then_some(Next::Refresh);
        }
        if let Some(cutout) = s.tools.cutout.take() {
            return Some(Next::Cutout(cutout));
        }
        if let Some(summary) = s.tools.summary.take() {
            return Some(Next::Summary(summary));
        }
        if !s.painted {
            return None;
        }
        s.tools.verb.take().map(|(resize, files)| Next::Verb(resize, files))
    })
    .flatten();
    match next {
        Some(Next::Refresh) => sheet::refresh(hwnd, Change::Refresh),
        Some(Next::Cutout(cutout)) => show_cutout(hwnd, cutout),
        Some(Next::Summary(summary)) => show_summary(hwnd, summary),
        Some(Next::Verb(resize, files)) => batch(hwnd, files, resize),
        None => {}
    }
}

/// The request again, with the edits and snapshots of this moment. A save
/// can start while a sheet or dialog is open.
fn fresh(s: &State, request: &Request) -> Option<Request> {
    (s.path.as_ref() == Some(&request.path)).then(|| Request {
        generation: s.generation,
        sessions: s.sessions.clone(),
        sources: s.opened_sources(),
        ..request.clone()
    })
}

/// A request that reads files from disk as they are, with no recipe.
fn plain_request(path: PathBuf) -> Request {
    Request { generation: 0, path, page: 0, delta: 0, width: 1, height: 1, sessions: HashMap::new(), sources: HashMap::new() }
}

#[derive(Default)]
struct Estimate {
    /// The file being estimated while its sheet is open.
    request: Option<Request>,
    id: u64,
    wanted: Option<(Vec<ImageEdit>, ExportOptions)>,
    due: Option<Instant>,
    result: Option<Result<u64, String>>,
}

impl Estimate {
    /// `id` keeps counting, so a late result never matches a new sheet.
    fn reset(&mut self, request: Option<Request>) {
        *self = Estimate { id: self.id, request, ..Default::default() };
    }
}

/// The size line for a sheet. A new setting starts a debounced estimate.
fn size_text(s: &mut State, extra: Vec<ImageEdit>, options: ExportOptions) -> String {
    let e = &mut s.tools.estimate;
    let wanted = Some((extra, options));
    if e.wanted != wanted {
        e.id += 1;
        e.wanted = wanted;
        e.result = None;
        e.due = Some(Instant::now() + ESTIMATE_DELAY);
    }
    match &e.result {
        None => "Estimating the file size...".into(),
        Some(Ok(bytes)) => format!("File size: about {}.", file_size(*bytes)),
        Some(Err(error)) => error.clone(),
    }
}

fn send_estimate(s: &mut State) {
    let e = &mut s.tools.estimate;
    if !e.due.is_some_and(|due| Instant::now() >= due) {
        return;
    }
    e.due = None;
    let (Some(request), Some((extra, options))) = (e.request.clone(), e.wanted.clone()) else {
        return;
    };
    let id = e.id;
    let Some(request) = fresh(s, &request) else {
        return;
    };
    if !s.send(Job::Tool(request, Task::Estimate { id, extra, options })) {
        s.tools.estimate.result = Some(Err("The file size cannot be estimated.".into()));
        s.tools.refresh = true;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Output {
    Keep,
    Image(ImageFormat),
    Pdf,
}

/// Output choices: the format, its label, and its file extension.
fn outputs(keep: bool) -> Vec<(Output, &'static str, &'static str)> {
    let mut list = Vec::new();
    if keep {
        list.push((Output::Keep, "Keep the format", ""));
    }
    list.extend([
        (Output::Image(ImageFormat::Jpeg), "JPEG", "jpg"),
        (Output::Image(ImageFormat::Png), "PNG", "png"),
        (Output::Image(ImageFormat::WebP), "WebP", "webp"),
        (Output::Image(ImageFormat::Tiff), "TIFF", "tif"),
    ]);
    if heic_available() {
        list.push((Output::Image(ImageFormat::Heic), "HEIC", "heic"));
    }
    list.push((Output::Pdf, "PDF", "pdf"));
    list
}

/// Checked once: the test encode loads Media Foundation.
fn heic_available() -> bool {
    static HEIC: OnceLock<bool> = OnceLock::new();
    *HEIC.get_or_init(crate::imaging::heic_encode_available)
}

fn lossy(output: Output) -> bool {
    matches!(output, Output::Image(ImageFormat::Jpeg | ImageFormat::WebP | ImageFormat::Heic))
}

/// Quality 100 saves WebP lossless.
fn options(format: ImageFormat, quality: usize) -> ExportOptions {
    let quality = quality.clamp(1, 100);
    ExportOptions { format, quality: quality as f32 / 100.0, lossless: format == ImageFormat::WebP && quality == 100 }
}

/// A free "name (edited).ext" path beside `path`, for a save dialog.
fn suggested(path: &Path, extension: &str) -> String {
    disk::copy_name(path, Some(OsStr::new(extension)))
        .map(|p| super::app::display_path(&p))
        .unwrap_or_else(|| format!("Edited copy.{extension}"))
}

/// A positive number. A comma also works as the decimal mark.
fn number(text: &str) -> Option<f64> {
    text.trim().replace(',', ".").parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0)
}

/// "50", or "33.33" when the value is not whole.
fn decimal(value: f64) -> String {
    if (value - value.round()).abs() < 0.005 {
        format!("{}", value.round() as u64)
    } else {
        format!("{value:.2}").trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Save a copy of an image: format, quality, and the file size before saving.
pub(super) unsafe fn export(hwnd: HWND, request: Request, width: u32, height: u32) {
    let list = outputs(false);
    let own = crate::imaging::format_for(&request.path).map(Output::Image);
    let chosen = list.iter().position(|o| Some(o.0) == own).unwrap_or(1);
    let labels: Vec<&str> = list.iter().map(|o| o.1).collect();
    let controls = vec![Control::choice("Format", &labels, chosen), Control::slider("Quality", 92)];
    with_state(|s| s.tools.estimate.reset(Some(request.clone())));
    let choices = list.clone();
    let live: sheet::Live = Box::new(move |s, _| {
        let output = choices[sheet::control(s, 0).min(choices.len() - 1)].0;
        let quality = sheet::control(s, 1);
        sheet::enable_control(s, 1, lossy(output));
        let detail = match output {
            Output::Image(format) => size_text(s, Vec::new(), options(format, quality)),
            _ => "The image becomes one PDF page.".into(),
        };
        let lossless = if output == Output::Image(ImageFormat::WebP) && quality == 100 { " Quality 100 saves lossless WebP." } else { "" };
        sheet::set_message(s, format!("{width} × {height} pixels. {detail}{lossless}"));
    });
    let answer = sheet::ask_live(hwnd, "Save a copy", &[], controls, &["Save", "Cancel"], live);
    with_state(|s| s.tools.estimate.reset(None));
    let Some((_, _, values)) = answer else {
        return;
    };
    let (output, label, extension) = list[values[0].min(list.len() - 1)];
    let Some(target) = files::save_as(hwnd, &suggested(&request.path, extension), &format!("{label} file"), extension) else {
        return;
    };
    let Some(request) = with_state(|s| fresh(s, &request)).flatten() else {
        return;
    };
    let job = match output {
        Output::Image(format) => Job::Tool(request, Task::Export { output: target, options: options(format, values[1]) }),
        _ => Job::Save(request, target, SaveKind::Copy),
    };
    start_export(hwnd, job, "Saving a new copy...");
}

/// The new size in pixels from the resize fields.
fn resize_target(source: [u32; 2], percent: bool, width: &str, height: &str) -> Result<[u32; 2], String> {
    let mut size = [0u32; 2];
    for (i, text) in [width, height].into_iter().enumerate() {
        let value = number(text).ok_or("Enter a width and height above 0.")?;
        let pixels = if percent { source[i] as f64 * value / 100.0 } else { value };
        if pixels > 100_000.0 {
            return Err("Use at most 100,000 pixels on each side.".into());
        }
        size[i] = pixels.round().max(1.0) as u32;
    }
    if size[0] as u64 * size[1] as u64 > 200_000_000 {
        return Err("Use at most 200 million pixels in total.".into());
    }
    Ok(size)
}

/// Resize by pixels or percent, with the aspect ratio locked or free.
pub(super) unsafe fn resize(hwnd: HWND, request: Request, width: u32, height: u32) {
    let source = [width.max(1), height.max(1)];
    // Autosave and Save a copy write the file's own format with these options.
    let saved_as = crate::imaging::format_for(&request.path).map(|format| ExportOptions { format, quality: 0.92, lossless: true });
    let controls = vec![Control::choice("Size in", &["Pixels", "Percent"], 0), Control::toggle("Keep the aspect ratio", true)];
    let fields = [("Width", width.to_string()), ("Height", height.to_string())];
    with_state(|s| s.tools.estimate.reset(Some(request.clone())));
    let live: sheet::Live = Box::new(move |s, change| {
        let percent = sheet::control(s, 0) == 1;
        let locked = sheet::control(s, 1) == 1;
        if change == Change::Control(0) {
            for i in 0..2 {
                if let Some(value) = number(&sheet::field(s, i)) {
                    let converted = if percent { value * 100.0 / source[i] as f64 } else { value * source[i] as f64 / 100.0 };
                    sheet::set_field(s, i, &decimal(converted));
                }
            }
        }
        let leader = match change {
            Change::Field(i) if locked => Some(i.min(1)),
            Change::Control(1) if locked => Some(0),
            _ => None,
        };
        if let Some(i) = leader {
            if let Some(value) = number(&sheet::field(s, i)) {
                let j = 1 - i;
                let other = if percent { value } else { (value * source[j] as f64 / source[i] as f64).round().max(1.0) };
                sheet::set_field(s, j, &decimal(other));
            }
        }
        let message = match resize_target(source, percent, &sheet::field(s, 0), &sheet::field(s, 1)) {
            Ok([w, h]) => {
                let size = saved_as.map_or(String::new(), |options| {
                    format!(" {}", size_text(s, vec![ImageEdit::Resize { width: w, height: h }], options))
                });
                format!("New size: {w} × {h} pixels.{size}")
            }
            Err(error) => error,
        };
        sheet::set_message(s, message);
    });
    let answer = sheet::ask_live(hwnd, "Resize image", &fields, controls, &["Resize", "Cancel"], live);
    with_state(|s| s.tools.estimate.reset(None));
    let Some((_, values, controls)) = answer else {
        return;
    };
    match resize_target(source, controls[0] == 1, &values[0], &values[1]) {
        Ok(size) if size == source => {}
        Ok([w, h]) => {
            if with_state(|s| s.path.as_ref() == Some(&request.path)) == Some(true) {
                actions::edit(hwnd, |_, edits| edits.image.push(ImageEdit::Resize { width: w, height: h }));
            }
        }
        Err(error) => sheet::alert(hwnd, "Resize image", &error),
    }
}

const RESIZES: [&str; 4] = ["Do not resize", "Percent", "Longest side", "Width and height"];
/// The size field's label and starting text for each resize choice.
const SIZES: [(&str, &str); 4] =
    [("Size", ""), ("Percent", "50"), ("Longest side in pixels", "1920"), ("Width x height in pixels", "1920 x 1080")];
const ROTATIONS: [&str; 4] = ["Do not rotate", "Right 90°", "180°", "Left 90°"];

/// "1920 x 1080" as two sizes from 1 to 100,000.
fn pair(text: &str) -> Option<[u32; 2]> {
    let parts: Vec<u32> = text
        .split(|c: char| matches!(c, 'x' | 'X' | '×' | '*' | ',') || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<u32>().ok().filter(|v| (1..=100_000).contains(v)))
        .collect::<Option<_>>()?;
    (parts.len() == 2).then(|| [parts[0], parts[1]])
}

fn batch_job(output: Output, quality: usize, mode: usize, turns: usize, size: &str) -> Result<BatchJob, String> {
    let resize = match mode {
        0 => None,
        1 => Some(BatchResize::Percent(number(size).filter(|v| *v <= 10_000.0).ok_or("Enter a percent above 0.")? as f32)),
        2 => Some(BatchResize::MaxEdge(
            number(size).filter(|v| *v <= 100_000.0).ok_or("Enter the longest side in pixels, from 1 to 100,000.")?.round().max(1.0)
                as u32,
        )),
        _ => {
            let [width, height] = pair(size).ok_or("Enter a width and height in pixels, such as 1920 x 1080.")?;
            Some(BatchResize::Pixels { width, height })
        }
    };
    if output == Output::Pdf && resize.is_some() {
        return Err("PDF pages keep each image's size. Choose Do not resize to make PDFs.".into());
    }
    let options = match output {
        Output::Image(format) => Some(options(format, quality)),
        _ => None,
    };
    Ok(BatchJob { quarter_turns: (turns % 4) as u8, resize, options })
}

fn is_image(path: &Path) -> bool {
    crate::imaging::format_for(path).is_some() || path.extension().is_some_and(|e| e.eq_ignore_ascii_case("gif"))
}

/// Every image in the folder of `path`.
pub(super) unsafe fn batch_folder(hwnd: HWND, path: &Path) {
    let folder = path.parent().unwrap_or(Path::new("."));
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder)
        .map(|entries| entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_file() && is_image(p)).collect())
        .unwrap_or_default();
    files.sort();
    batch(hwnd, files, false);
}

/// Convert, resize, and rotate many images in one action. `resize` starts
/// with resizing chosen, for the Explorer Resize verb.
pub(super) unsafe fn batch(hwnd: HWND, files: Vec<PathBuf>, resize: bool) {
    if files.is_empty() {
        sheet::alert(hwnd, "Convert images", "There are no images to convert.");
        return;
    }
    let list = outputs(true);
    let labels: Vec<&str> = list.iter().map(|o| o.1).collect();
    let jpeg = list.iter().position(|o| o.0 == Output::Image(ImageFormat::Jpeg)).unwrap_or(0);
    let mut values = vec![if resize { 0 } else { jpeg }, 92, usize::from(resize), 0];
    let mut size = SIZES[values[2]].1.to_string();
    let count = files.len();
    loop {
        let controls = vec![
            Control::choice("Format", &labels, values[0]),
            Control::slider("Quality", values[1]),
            Control::choice("Resize", &RESIZES, values[2]),
            Control::choice("Rotate", &ROTATIONS, values[3]),
        ];
        let choices = list.clone();
        let live: sheet::Live = Box::new(move |s, change| {
            let output = choices[sheet::control(s, 0).min(choices.len() - 1)].0;
            let mode = sheet::control(s, 2).min(3);
            sheet::enable_control(s, 1, lossy(output));
            sheet::set_field_label(s, 0, SIZES[mode].0);
            if change == Change::Control(2) {
                sheet::set_field(s, 0, SIZES[mode].1);
            }
            let images = if count == 1 { "1 image".to_string() } else { format!("{count} images") };
            let message = match batch_job(output, sheet::control(s, 1), mode, sheet::control(s, 3), &sheet::field(s, 0)) {
                Ok(_) => format!("{images}. New files go to a folder you choose. Existing files are never replaced."),
                Err(error) => error,
            };
            sheet::set_message(s, message);
        });
        let fields = [(SIZES[values[2].min(3)].0, size.clone())];
        let Some((_, text, chosen)) = sheet::ask_live(hwnd, "Convert images", &fields, controls, &["Convert", "Cancel"], live) else {
            return;
        };
        values = chosen;
        size = text.into_iter().next().unwrap_or_default();
        let output = list[values[0].min(list.len() - 1)].0;
        match batch_job(output, values[1], values[2], values[3], &size) {
            Ok(job) => {
                start_batch(hwnd, files, output, job);
                return;
            }
            Err(error) => sheet::alert(hwnd, "Convert images", &error),
        }
    }
}

unsafe fn start_batch(hwnd: HWND, files: Vec<PathBuf>, output: Output, job: BatchJob) {
    if with_state(|s| s.exporting) != Some(false) {
        sheet::alert(hwnd, "Convert images", "Wait for the current save or conversion to finish, then try again.");
        return;
    }
    let Some(folder) = files::folder(hwnd) else {
        return;
    };
    let count = files.len();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut request = plain_request(files[0].clone());
    let job = if output == Output::Pdf {
        // PDFs are made on the document worker, which owns PDFium.
        let turns = vec![ImageEdit::RotateRight; job.quarter_turns as usize % 4];
        request.sessions.insert(request.path.clone(), Edits { image: turns, ..Edits::default() });
        Job::Batch(request, folder, "pdf".into(), cancel.clone(), Some(files))
    } else {
        Job::Tool(request, Task::Batch { files, folder, job, cancel: cancel.clone() })
    };
    with_state(|s| s.cancel = Some(cancel));
    start_export(hwnd, job, &format!("Converting {count} images. Press Escape to cancel."));
}

unsafe fn show_summary(hwnd: HWND, summary: BatchSummary) {
    let files = if summary.saved == 1 { "file" } else { "files" };
    let mut message = format!("{} new {files} in {}.", summary.saved, super::app::display_path(&summary.folder));
    if !summary.failed.is_empty() {
        message += &format!("\n\n{} could not be converted:", summary.failed.len());
        for line in summary.failed.iter().take(8) {
            message += &format!("\n{line}");
        }
        if summary.failed.len() > 8 {
            message += &format!("\nAnd {} more.", summary.failed.len() - 8);
        }
    }
    if summary.skipped > 0 {
        message += &format!("\n\n{} not converted because you canceled.", summary.skipped);
    }
    let title = if summary.skipped > 0 { "Conversion canceled" } else { "Conversion finished" };
    let open = sheet::ask(hwnd, title, &message, &[], false, &["Open folder", "Close"], 1, Some(0)).is_some();
    if open {
        let folder: Vec<u16> = summary.folder.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
        ShellExecuteW(Some(hwnd), w!("open"), PCWSTR(folder.as_ptr()), None, None, SW_SHOWNORMAL);
    }
}

pub(super) unsafe fn remove_background(hwnd: HWND, request: Request) {
    start_export(hwnd, Job::Tool(request, Task::Cutout), "Removing the background on this PC...");
}

/// The result stays in memory until it is saved or the sheet closes.
unsafe fn show_cutout(hwnd: HWND, cutout: Cutout) {
    let mut message = format!(
        "Done in {:.1} seconds. The {} × {} pixel result has a transparent background.",
        cutout.seconds, cutout.frame.width, cutout.frame.height
    );
    loop {
        let buttons = ["Save as PNG", "Copy", "Close"];
        let Some((button, _)) = sheet::ask(hwnd, "Background removed", &message, &[], false, &buttons, 2, Some(0)) else {
            return;
        };
        if button == 1 {
            let job = Job::Tool(plain_request(cutout.path.clone()), Task::ClipboardFrame { frame: cutout.frame.clone() });
            with_state(|s| {
                s.status = "Copying...".into();
                s.send(job)
            });
            message = "Copying it to the clipboard. You can also save it as a PNG.".into();
            continue;
        }
        let Some(output) = files::save_as(hwnd, &suggested(&cutout.path, "png"), "PNG image", "png") else {
            continue;
        };
        let job = Job::Tool(plain_request(cutout.path), Task::SaveFrame { frame: cutout.frame, output });
        start_export(hwnd, job, "Saving the PNG...");
        return;
    }
}

/// Edges that a press at (x, y) grabs: a corner or edge handle, all four
/// inside the box to move it, or None outside it.
fn grab_at(rect: [f32; 4], image: Rect, x: f32, y: f32, reach: f32) -> Option<[bool; 4]> {
    let left = image.x0 + rect[0] * image.width();
    let top = image.y0 + rect[1] * image.height();
    let right = image.x0 + rect[2] * image.width();
    let bottom = image.y0 + rect[3] * image.height();
    if x < left - reach || x > right + reach || y < top - reach || y > bottom + reach {
        return None;
    }
    let near = |a: f32, b: f32| (a - b).abs() <= reach;
    let edges = [near(x, left), near(y, top), near(x, right) && !near(x, left), near(y, bottom) && !near(y, top)];
    if edges.contains(&true) {
        Some(edges)
    } else if x > left && x < right && y > top && y < bottom {
        Some([true; 4])
    } else {
        None
    }
}

/// The box after dragging by `delta`, in image fractions. A moved box stays
/// whole inside the image; an edge dragged past its opposite flips the box.
fn dragged(grab: Grab, delta: [f32; 2]) -> [f32; 4] {
    let s = grab.start;
    if grab.edges == [true; 4] {
        let dx = delta[0].clamp(-s[0], 1.0 - s[2]);
        let dy = delta[1].clamp(-s[1], 1.0 - s[3]);
        return [s[0] + dx, s[1] + dy, s[2] + dx, s[3] + dy];
    }
    let mut r = s;
    for i in 0..4 {
        if grab.edges[i] {
            r[i] = (s[i] + delta[i % 2]).clamp(0.0, 1.0);
        }
    }
    [r[0].min(r[2]), r[1].min(r[3]), r[0].max(r[2]), r[1].max(r[3])]
}

/// Pointer input in image crop mode. Returns true to repaint.
pub(super) unsafe fn crop_pointer(hwnd: HWND, state: &mut State, e: &PointerEvent) -> bool {
    let image = state.image_rect;
    if image.width() <= 0.0 || image.height() <= 0.0 {
        return false;
    }
    let at = [(e.x - image.x0) / image.width(), (e.y - image.y0) / image.height()];
    match e.phase {
        Phase::Down => {
            if state.pending || state.render_failed || state.frame.is_none() {
                return false;
            }
            let reach = 10.0 * state.scale;
            let grabbed = state.tools.crop.and_then(|c| grab_at(c.rect, image, e.x, e.y, reach).map(|edges| (c.rect, edges)));
            let (start, edges) = grabbed.unwrap_or_else(|| {
                let p = [at[0].clamp(0.0, 1.0), at[1].clamp(0.0, 1.0)];
                ([p[0], p[1], p[0], p[1]], [false, false, true, true])
            });
            state.tools.crop = Some(CropBox { rect: start, grab: Some(Grab { edges, from: at, start }) });
            state.drag = Some((e.x, e.y));
            if e.kind == PointerKind::Mouse {
                SetCapture(hwnd);
            }
            true
        }
        Phase::Move => {
            let Some(crop) = state.tools.crop.as_mut() else {
                return false;
            };
            let Some(grab) = crop.grab else {
                return false;
            };
            crop.rect = dragged(grab, [at[0] - grab.from[0], at[1] - grab.from[1]]);
            true
        }
        Phase::Up | Phase::Cancel => {
            let _ = ReleaseCapture();
            state.drag = None;
            let least = 4.0 * state.scale;
            if let Some(crop) = state.tools.crop.as_mut() {
                if let (Phase::Cancel, Some(grab)) = (e.phase, crop.grab) {
                    crop.rect = grab.start;
                }
                crop.grab = None;
                let r = crop.rect;
                if (r[2] - r[0]) * image.width() < least || (r[3] - r[1]) * image.height() < least {
                    state.tools.crop = None;
                }
            }
            state.status = match (state.tools.select, state.tools.crop) {
                (Some(_), _) => SELECT_HINT,
                (None, Some(_)) => ADJUST_HINT,
                (None, None) => CROP_HINT,
            }
            .into();
            true
        }
    }
}

/// Turns the rectangular or elliptical selection tool on, or off when it
/// is already on.
pub(super) unsafe fn select(hwnd: HWND, ellipse: bool) {
    with_state(|s| {
        let on = s.crop && s.tools.select == Some(ellipse);
        s.markup = None;
        s.signature = None;
        s.marks.deselect();
        s.zoom_select = false;
        s.tools.crop = None;
        s.crop = !on;
        s.tools.select = (!on).then_some(ellipse);
        if !on {
            s.set_markup(true);
        }
        s.status = if on { "Selection is off." } else { SELECT_HINT }.into();
    });
    invalidate(hwnd);
}

/// True while a selection on the image can be cropped to or cleared.
pub(super) fn has_selection(state: &State) -> bool {
    state.crop && state.pdf.is_none() && state.tools.select.is_some() && state.tools.crop.is_some()
}

/// Crops to the selection, or clears it when `clear`. An elliptical crop
/// keeps the ellipse and clears the corners.
pub(super) unsafe fn apply(hwnd: HWND, clear: bool) {
    let Some((r, ellipse, white)) = with_state(|s| {
        let r = s.tools.crop?.rect;
        let ellipse = s.tools.select?;
        let alpha = matches!(s.path.as_deref().and_then(crate::imaging::format_for), Some(ImageFormat::Png | ImageFormat::WebP | ImageFormat::Tiff));
        s.crop = false;
        s.tools.crop = None;
        s.tools.select = None;
        s.drag = None;
        Some((r, ellipse, !alpha))
    })
    .flatten() else {
        return;
    };
    let _ = ReleaseCapture();
    actions::edit(hwnd, |_, edits| {
        if clear {
            edits.image.push(ImageEdit::Clear { rect: r, ellipse, outside: false, white });
            return;
        }
        edits.image.push(ImageEdit::Crop { left: r[0], top: r[1], right: r[2], bottom: r[3] });
        if ellipse {
            edits.image.push(ImageEdit::Clear { rect: [0.0, 0.0, 1.0, 1.0], ellipse, outside: true, white });
        }
    });
    invalidate(hwnd);
}

/// Enter crops to the box; Escape leaves crop mode; Delete clears a
/// selection. Returns true when handled.
pub(super) unsafe fn crop_key(hwnd: HWND, vk: u16) -> bool {
    let key = VIRTUAL_KEY(vk);
    if matches!(key, VK_DELETE | VK_BACK) && with_state(|s| has_selection(s)) == Some(true) {
        apply(hwnd, true);
        return true;
    }
    if !matches!(key, VK_RETURN | VK_ESCAPE) {
        return false;
    }
    let Some((active, rect)) = with_state(|s| (s.crop && s.pdf.is_none(), s.tools.crop.map(|c| c.rect))) else {
        return false;
    };
    if !active {
        return false;
    }
    if key == VK_RETURN && rect.is_none() {
        with_state(|s| s.status = if s.tools.select.is_some() { SELECT_HINT } else { CROP_HINT }.into());
        invalidate(hwnd);
        return true;
    }
    if key == VK_RETURN && with_state(|s| s.tools.select.is_some()) == Some(true) {
        apply(hwnd, false);
        return true;
    }
    with_state(|s| {
        s.crop = false;
        s.tools.crop = None;
        s.tools.select = None;
        s.drag = None;
        s.status = if key == VK_ESCAPE { "Crop canceled." } else { "Cropping..." }.into();
    });
    let _ = ReleaseCapture();
    if let (VK_RETURN, Some(r)) = (key, rect) {
        if r != [0.0, 0.0, 1.0, 1.0] {
            actions::edit(hwnd, |_, edits| edits.image.push(ImageEdit::Crop { left: r[0], top: r[1], right: r[2], bottom: r[3] }));
        }
    }
    invalidate(hwnd);
    true
}

/// Shades the area outside the crop box and draws the box with handles. A
/// selection is drawn as its outline only.
pub(super) fn paint_crop(p: &Painter, state: &State) {
    let Some(crop) = state.tools.crop.filter(|_| state.crop && state.pdf.is_none()) else {
        return;
    };
    let (image, s, theme) = (state.image_rect, state.scale, state.theme);
    let r = crop.rect;
    let b = Rect {
        x0: image.x0 + r[0] * image.width(),
        y0: image.y0 + r[1] * image.height(),
        x1: image.x0 + r[2] * image.width(),
        y1: image.y0 + r[3] * image.height(),
    };
    match state.tools.select {
        Some(true) => p.stroke_ellipse(b, theme.accent, 2.0 * s),
        Some(false) => p.stroke_round(b, 0.0, theme.accent, 2.0 * s),
        None => {
            let shade = Rgba(0.0, 0.0, 0.0, 0.5);
            p.fill(Rect { y1: b.y0, ..image }, shade);
            p.fill(Rect { y0: b.y1, ..image }, shade);
            p.fill(Rect { x1: b.x0, y0: b.y0, y1: b.y1, ..image }, shade);
            p.fill(Rect { x0: b.x1, y0: b.y0, y1: b.y1, ..image }, shade);
            p.stroke_round(b.inset(-2.0 * s), 0.0, theme.accent, 2.0 * s);
        }
    }
    let side = 10.0 * s;
    let (mx, my) = ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
    for (x, y) in [(b.x0, b.y0), (mx, b.y0), (b.x1, b.y0), (b.x1, my), (b.x1, b.y1), (mx, b.y1), (b.x0, b.y1), (b.x0, my)] {
        let handle = Rect::new(x - side / 2.0, y - side / 2.0, side, side);
        p.fill_round(handle, 2.0 * s, Rgba(1.0, 1.0, 1.0, 1.0));
        p.stroke_round(handle, 2.0 * s, theme.accent, 2.0 * s);
    }
}
