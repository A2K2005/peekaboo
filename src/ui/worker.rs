//! The document worker owns every PDFium call and the WIC viewing path. A
//! second task worker runs jobs that never touch PDFium (OCR, background
//! removal, image exports, and image batches), so they never delay page
//! rendering. Both run the same `worker` loop; the PDFium engine guard in
//! pdf.rs refuses a second engine, so the task worker cannot load PDFium.
use crate::model::{Frame, ImageEdit, PdfEdit};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    System::Com::*,
    UI::WindowsAndMessaging::{PostMessageW, WM_APP},
};

/// Posted after each worker event so results reach the window at once,
/// not on the next timer tick.
pub(super) const WM_APP_WAKE: u32 = WM_APP + 1;

#[derive(Clone)]
pub(super) struct Events {
    sender: mpsc::Sender<Event>,
    window: isize,
}
impl Events {
    fn send(&self, event: Event) -> Result<(), mpsc::SendError<Event>> {
        self.sender.send(event)?;
        unsafe {
            let _ = PostMessageW(Some(HWND(self.window as *mut _)), WM_APP_WAKE, WPARAM(0), LPARAM(0));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum SaveKind {
    Copy,
    ExtractPage,
    Merge(PathBuf),
}

#[derive(Clone, Default, Debug)]
pub(super) struct Edits {
    pub(super) image: Vec<ImageEdit>,
    pub(super) pdf: Vec<PdfEdit>,
    pub(super) dirty: bool,
}
#[derive(Clone)]
pub(super) struct Request {
    pub(super) generation: u64,
    pub(super) path: PathBuf,
    pub(super) page: u32,
    pub(super) delta: i32,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) sessions: HashMap<PathBuf, Edits>,
}
pub(super) enum Job {
    Render(Request),
    Save(Request, PathBuf, SaveKind),
    Text(Request),
    Find(Request, String),
    Fields(Request),
    Background(Request, PathBuf),
    Print(Request, crate::printing::PrintJob),
    Batch(
        Request,
        PathBuf,
        String,
        Arc<AtomicBool>,
        Option<Vec<PathBuf>>,
    ),
    Password(Request, String),
}
pub(super) enum Event {
    Render(Completed),
    Saved(PathBuf, Edits, bool, std::result::Result<(), String>),
    Text(u64, PathBuf, std::result::Result<String, String>),
    Found(u64, std::result::Result<Option<u32>, String>),
    Fields(u64, std::result::Result<Vec<crate::pdf::FormField>, String>),
    Background(PathBuf, std::result::Result<String, String>),
    Finished(std::result::Result<String, String>),
    Progress(String),
}
pub(super) struct Completed {
    pub(super) generation: u64,
    pub(super) path: PathBuf,
    pub(super) page: u32,
    pub(super) navigation: bool,
    pub(super) result: std::result::Result<Frame, String>,
}

pub(super) fn is_pdf(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

pub(super) fn sibling(path: &Path, delta: i32) -> std::result::Result<PathBuf, String> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("Cannot read this folder: {e}"))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|e| {
                matches!(
                    e.to_string_lossy().to_lowercase().as_str(),
                    "jpg"
                        | "jpeg"
                        | "png"
                        | "gif"
                        | "tif"
                        | "tiff"
                        | "bmp"
                        | "webp"
                        | "heic"
                        | "heif"
                )
            })
        })
        .collect();
    files.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    let index = files
        .iter()
        .position(|p| p == path)
        .ok_or("This image is no longer in its folder.")?;
    let next = index as i64 + delta as i64;
    files
        .get(usize::try_from(next).unwrap_or(usize::MAX))
        .cloned()
        .ok_or("No more images in this direction.".into())
}

fn worker(receiver: mpsc::Receiver<Job>, sender: Events) {
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_MULTITHREADED).ok();
        let mut engine = None;
        while let Ok(job) = receiver.recv() {
            let job = match job {
                Job::Password(request, password) => {
                    if engine.is_none() {
                        engine = crate::pdf::PdfEngine::new().ok();
                    }
                    let result = match engine.as_mut() {
                        Some(engine) => engine.set_password(&request.path, password),
                        None => Err("PDF support could not load.".into()),
                    };
                    if let Err(error) = result {
                        let _ = sender.send(Event::Render(Completed {
                            generation: request.generation,
                            path: request.path,
                            page: request.page,
                            navigation: false,
                            result: Err(error),
                        }));
                        continue;
                    }
                    Job::Render(request)
                }
                other => other,
            };
            let mut request = match &job {
                Job::Render(r)
                | Job::Save(r, _, _)
                | Job::Text(r)
                | Job::Find(r, _)
                | Job::Fields(r)
                | Job::Background(r, _)
                | Job::Print(r, _)
                | Job::Batch(r, _, _, _, _)
                | Job::Password(r, _) => r.clone(),
            };
            if !matches!(job, Job::Render(_)) {
                let edits = request
                    .sessions
                    .get(&request.path)
                    .cloned()
                    .unwrap_or_default();
                if is_pdf(&request.path) && engine.is_none() {
                    engine = crate::pdf::PdfEngine::new().ok();
                }
                let event = match job {
                    Job::Save(_, output, kind) => {
                        let result = if is_pdf(&request.path) {
                            match engine.as_mut() {
                                None => Err("PDF support could not load.".into()),
                                Some(engine) => match &kind {
                                    SaveKind::ExtractPage => engine.extract_page(
                                        &request.path,
                                        &output,
                                        request.page,
                                        &edits.pdf,
                                    ),
                                    SaveKind::Merge(other) => {
                                        engine.merge(&request.path, other, &output, &edits.pdf)
                                    }
                                    SaveKind::Copy => engine.save_copy(&request.path, &output, &edits.pdf),
                                },
                            }
                        } else if is_pdf(&output) {
                            if engine.is_none() {
                                engine = crate::pdf::PdfEngine::new().ok();
                            }
                            match engine.as_mut() {
                                Some(engine) => crate::imaging::decode_edited(
                                    &request.path,
                                    u32::MAX,
                                    u32::MAX,
                                    &edits.image,
                                )
                                .and_then(|frame| engine.create_from_image(&frame, &output)),
                                None => Err("PDF support could not load.".into()),
                            }
                        } else {
                            crate::imaging::export(&request.path, &output, &edits.image)
                        };
                        Event::Saved(
                            request.path,
                            edits,
                            kind != SaveKind::ExtractPage,
                            result.map_err(|e| format!("Could not save {}: {e}", output.display())),
                        )
                    }
                    Job::Text(_) => Event::Text(
                        request.generation,
                        request.path.clone(),
                        if is_pdf(&request.path) {
                            match engine.as_mut() {
                                Some(e) => {
                                    e.page_text_edited(&request.path, request.page, &edits.pdf)
                                }
                                None => Err("Open a PDF first.".into()),
                            }
                        } else {
                            crate::ocr::recognize(&request.path, &edits.image)
                        },
                    ),
                    Job::Find(_, query) => Event::Found(
                        request.generation,
                        match engine.as_mut() {
                            Some(e) => e.find_edited(
                                &request.path,
                                &query,
                                request.page.saturating_add(1),
                                &edits.pdf,
                            ),
                            None => Err("Open a PDF first.".into()),
                        },
                    ),
                    Job::Fields(_) => Event::Fields(
                        request.generation,
                        match engine.as_mut() {
                            Some(e) => e.form_fields(&request.path, request.page, &edits.pdf),
                            None => Err("Open a PDF first.".into()),
                        },
                    ),
                    Job::Background(_, output) => {
                        let result =
                            crate::background::remove(&request.path, &output, &edits.image);
                        Event::Background(output, result)
                    }
                    Job::Print(_, job) => {
                        if engine.is_none() {
                            engine = crate::pdf::PdfEngine::new().ok();
                        }
                        Event::Finished(match engine.as_mut() {
                            Some(engine) => crate::printing::print(
                                job,
                                engine,
                                &request.path,
                                &edits.pdf,
                                &edits.image,
                                is_pdf(&request.path),
                            )
                            .map(|_| "Print job sent to Windows.".into()),
                            None => Err("PDF support could not load.".into()),
                        })
                    }
                    Job::Batch(_, folder, extension, cancel, selected) => {
                        let mut good = 0;
                        let mut errors = Vec::new();
                        let files = if let Some(selected) = selected {
                            Ok(selected)
                        } else {
                            std::fs::read_dir(request.path.parent().unwrap_or(Path::new("."))).map(
                                |entries| {
                                    entries
                                        .filter_map(|e| e.ok())
                                        .map(|e| e.path())
                                        .filter(|p| {
                                            p.is_file()
                                                && p.extension().is_some_and(|e| {
                                                    matches!(
                                                        e.to_string_lossy().to_lowercase().as_str(),
                                                        "png"
                                                            | "jpg"
                                                            | "jpeg"
                                                            | "bmp"
                                                            | "tif"
                                                            | "tiff"
                                                            | "gif"
                                                            | "webp"
                                                            | "heic"
                                                            | "heif"
                                                    )
                                                })
                                        })
                                        .collect::<Vec<_>>()
                                },
                            )
                        };
                        match files {
                            Err(error) => {
                                Event::Finished(Err(format!("Cannot read this folder: {error}")))
                            }
                            Ok(mut files) => {
                                files.sort();
                                let total = files.len();
                                for (index, path) in files.iter().enumerate() {
                                    if cancel.load(Ordering::Acquire) {
                                        break;
                                    }
                                    let _=sender.send(Event::Progress(format!("Converting image {} of {total}. Escape cancels after the current file.",index+1)));
                                    let mut output =
                                        folder.join(path.file_name().unwrap_or_default());
                                    output.set_extension(&extension);
                                    let result = if extension == "pdf" {
                                        if engine.is_none() {
                                            engine = crate::pdf::PdfEngine::new().ok();
                                        }
                                        match engine.as_mut() {
                                            Some(engine) => crate::imaging::decode_edited(
                                                path,
                                                u32::MAX,
                                                u32::MAX,
                                                &edits.image,
                                            )
                                            .and_then(|frame| {
                                                engine.create_from_image(&frame, &output)
                                            }),
                                            None => Err("PDF support could not load.".into()),
                                        }
                                    } else {
                                        crate::imaging::export(path, &output, &edits.image)
                                    };
                                    match result {
                                        Ok(()) => good += 1,
                                        Err(error) => errors.push(format!(
                                            "{}: {error}",
                                            path.file_name().unwrap_or_default().to_string_lossy()
                                        )),
                                    }
                                }
                                Event::Finished(Ok(format!(
                                    "Batch finished: {good} saved, {} failed, {} not processed.{}",
                                    errors.len(),
                                    total - good - errors.len(),
                                    if errors.is_empty() {
                                        String::new()
                                    } else {
                                        format!(" First error: {}", errors[0])
                                    }
                                )))
                            }
                        }
                    }
                    Job::Render(_) | Job::Password(_, _) => continue,
                };
                if sender.send(event).is_err() {
                    break;
                }
                continue;
            }
            let result = if let Err(error) = &initialized {
                Err(format!("Cannot initialize Windows imaging: {error}"))
            } else {
                let resolved = if request.delta != 0 {
                    sibling(&request.path, request.delta)
                } else {
                    Ok(request.path.clone())
                };
                match resolved {
                    Err(error) => Err(error),
                    Ok(path) => {
                        request.path = path;
                        if is_pdf(&request.path) {
                            if engine.is_none() {
                                engine = crate::pdf::PdfEngine::new().ok();
                            }
                            match engine.as_mut() {
                                Some(engine) => engine.render_edited(&request.path, request.page, request.width, request.height, &request.sessions.get(&request.path).cloned().unwrap_or_default().pdf),
                                None => Err("PDFium could not load. Install the verified pdfium.dll beside Preview and reopen the PDF.".into()),
                            }
                        } else {
                            crate::imaging::decode_edited(
                                &request.path,
                                request.width,
                                request.height,
                                &request
                                    .sessions
                                    .get(&request.path)
                                    .cloned()
                                    .unwrap_or_default()
                                    .image,
                            )
                        }
                    }
                }
            };
            if sender
                .send(Event::Render(Completed {
                    generation: request.generation,
                    path: request.path,
                    page: request.page,
                    navigation: request.delta != 0,
                    result,
                }))
                .is_err()
            {
                break;
            }
        }
        drop(engine);
        if initialized.is_ok() {
            CoUninitialize();
        }
    }
}

/// Jobs that never touch PDFium go to the task worker.
pub(super) fn runs_on_task_worker(job: &Job) -> bool {
    match job {
        Job::Text(request) => !is_pdf(&request.path),
        Job::Background(..) => true,
        Job::Batch(_, _, extension, _, _) => extension != "pdf",
        Job::Save(request, output, SaveKind::Copy) => !is_pdf(&request.path) && !is_pdf(output),
        _ => false,
    }
}

pub(super) struct Workers {
    document: mpsc::Sender<Job>,
    task: Option<mpsc::Sender<Job>>,
    events: Events,
    pub(super) receiver: mpsc::Receiver<Event>,
}

fn spawn(name: &str, events: Events) -> std::io::Result<mpsc::Sender<Job>> {
    let (send, receive) = mpsc::channel();
    std::thread::Builder::new().name(name.into()).spawn(move || worker(receive, events))?;
    Ok(send)
}

impl Workers {
    pub(super) fn start(window: HWND) -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let events = Events { sender, window: window.0 as isize };
        Ok(Self { document: spawn("document-render", events.clone())?, task: None, events, receiver })
    }
    /// Returns false when the worker has stopped.
    pub(super) fn send(&mut self, job: Job) -> bool {
        if !runs_on_task_worker(&job) {
            return self.document.send(job).is_ok();
        }
        if self.task.is_none() {
            // Started on first use, so launch pays nothing for it.
            self.task = spawn("task", self.events.clone()).ok();
        }
        self.task.as_ref().is_some_and(|task| task.send(job).is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_extension_is_case_insensitive() {
        assert!(is_pdf(Path::new("sample.PDF")));
        assert!(!is_pdf(Path::new("sample.png")));
    }

    #[test]
    fn sibling_navigation_filters_and_orders_images() {
        let folder = std::env::temp_dir().join(format!(
            "preview-shell-nav-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir(&folder).unwrap();
        for name in ["B.jpg", "a.PNG", "ignore.txt"] {
            std::fs::write(folder.join(name), b"fixture").unwrap();
        }
        assert_eq!(sibling(&folder.join("a.PNG"), 1).unwrap(), folder.join("B.jpg"));
        assert!(sibling(&folder.join("a.PNG"), -1).is_err());
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn pdfium_jobs_stay_on_the_document_worker() {
        let request = |path: &str| Request {
            generation: 1,
            path: PathBuf::from(path),
            page: 0,
            delta: 0,
            width: 1,
            height: 1,
            sessions: HashMap::new(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        assert!(runs_on_task_worker(&Job::Text(request("a.png"))), "OCR");
        assert!(!runs_on_task_worker(&Job::Text(request("a.pdf"))), "PDF text uses PDFium");
        assert!(runs_on_task_worker(&Job::Background(request("a.png"), PathBuf::from("b.png"))));
        assert!(runs_on_task_worker(&Job::Batch(request("a.png"), PathBuf::from("out"), "webp".into(), cancel.clone(), None)));
        assert!(!runs_on_task_worker(&Job::Batch(request("a.png"), PathBuf::from("out"), "pdf".into(), cancel, None)));
        assert!(runs_on_task_worker(&Job::Save(request("a.png"), PathBuf::from("b.jpg"), SaveKind::Copy)));
        assert!(!runs_on_task_worker(&Job::Save(request("a.png"), PathBuf::from("b.pdf"), SaveKind::Copy)));
        assert!(!runs_on_task_worker(&Job::Save(request("a.pdf"), PathBuf::from("b.pdf"), SaveKind::ExtractPage)));
        assert!(!runs_on_task_worker(&Job::Render(request("a.png"))));
        assert!(!runs_on_task_worker(&Job::Find(request("a.pdf"), "x".into())));
        assert!(!runs_on_task_worker(&Job::Fields(request("a.pdf"))));
    }
}
