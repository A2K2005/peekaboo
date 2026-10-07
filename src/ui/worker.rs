//! The document worker owns every PDFium call and the WIC viewing path. A
//! second task worker runs jobs that never touch PDFium (OCR, background
//! removal, image exports, and image batches), so they never delay page
//! rendering. Both run the same `worker` loop; the PDFium engine guard in
//! pdf.rs refuses a second engine, so the task worker cannot load PDFium.
//!
//! Jobs arrive in order on a channel. Page tiles, thumbnails, outline,
//! notes, and neighbor pre-decodes go through a shared `Queue` instead: the
//! window replaces the whole waiting list whenever the view changes, so the
//! newest request runs first and work for pages that scrolled away is
//! dropped before it starts (the SumatraPDF pattern in
//! docs/research/reference-architecture.md, Q2). Jobs run before queued work.
//! Long jobs (batches and printing) run one file or page per turn, after
//! both, so they never hold up the view.
use super::cache::Lru;
use crate::model::{Frame, ImageEdit, OutlineItem, PdfEdit, PdfMetadata};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
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
    /// Decodes an image (or its neighbor when `delta` is not 0).
    Render(Request),
    /// Opens a PDF and reads its page sizes.
    Open(Request),
    Metadata(Request),
    Save(Request, PathBuf, SaveKind),
    Text(Request),
    Find(Request, String),
    Fields(Request),
    Background(Request, PathBuf),
    Print(Request, crate::printing::PrintJob),
    Batch(Request, PathBuf, String, Arc<AtomicBool>, Option<Vec<PathBuf>>),
    Password(Request, String),
    /// New queued work: wakes a waiting worker.
    Wake,
}
pub(super) enum Event {
    Render(Completed),
    Pages(Opened),
    Done(Item, Outcome),
    Metadata(u64, std::result::Result<PdfMetadata, String>),
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
    /// The frame came from the neighbor pre-decode cache.
    pub(super) from_predecode: bool,
    pub(super) result: std::result::Result<Frame, String>,
}
/// Page sizes in points of an opened PDF, after `edits`.
pub(super) struct Opened {
    pub(super) generation: u64,
    pub(super) path: PathBuf,
    pub(super) page: u32,
    pub(super) edits: Arc<Vec<PdfEdit>>,
    pub(super) result: std::result::Result<Vec<[f32; 2]>, String>,
}

/// Queued work. `scale` is in pixels per point and `region` is
/// `[x, y, width, height]` in page pixels at that scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Work {
    Tile { page: u32, scale: u32, col: u32, row: u32 },
    Thumb { page: u32 },
    Outline,
    Notes { page: u32 },
    Predecode { delta: i32, width: u32, height: u32 },
}
/// `doc` identifies the file (`doc_id`), so two tabs never share tiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Key {
    pub(super) doc: u64,
    pub(super) work: Work,
}
#[derive(Clone, Debug)]
pub(super) struct Item {
    pub(super) key: Key,
    pub(super) path: PathBuf,
    pub(super) edits: Arc<Vec<PdfEdit>>,
    pub(super) scale: f32,
    pub(super) region: [u32; 4],
}
pub(super) enum Outcome {
    Frame(std::result::Result<Frame, String>),
    Outline(std::result::Result<Vec<OutlineItem>, String>),
    Notes(std::result::Result<Vec<Note>, String>),
    Predecoded,
}
/// An annotation with text, for the Notes tab.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Note {
    pub(super) page: u32,
    pub(super) kind: String,
    pub(super) text: String,
}

pub(super) fn doc_id(path: &Path) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}

/// Waiting work, most urgent first.
#[derive(Default)]
pub(super) struct Queue {
    items: VecDeque<Item>,
    /// Popped, and not yet delivered to the window. Never queued twice.
    taken: HashSet<Key>,
}
impl Queue {
    /// Replaces the waiting work. Returns true when the worker may be
    /// waiting on its channel and needs a wake-up.
    pub(super) fn set(&mut self, items: Vec<Item>) -> bool {
        let idle = self.items.is_empty();
        let taken = &self.taken;
        self.items = items.into_iter().filter(|i| !taken.contains(&i.key)).collect();
        idle && !self.items.is_empty()
    }
    pub(super) fn pop(&mut self) -> Option<Item> {
        let item = self.items.pop_front()?;
        self.taken.insert(item.key);
        Some(item)
    }
    pub(super) fn delivered(&mut self, key: &Key) {
        self.taken.remove(key);
    }
}

pub(super) fn is_pdf(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
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
                    "jpg" | "jpeg" | "png" | "gif" | "tif" | "tiff" | "bmp" | "webp" | "heic" | "heif"
                )
            })
        })
        .collect();
    files.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    let index = files.iter().position(|p| p == path).ok_or("This image is no longer in its folder.")?;
    let next = index as i64 + delta as i64;
    files.get(usize::try_from(next).unwrap_or(usize::MAX)).cloned().ok_or("No more images in this direction.".into())
}

/// Pre-decoded images by path and decode box. Holds the previous, current,
/// and next image at display size.
type Decoded = Lru<(PathBuf, u32, u32), Frame>;

/// A batch conversion in progress.
struct Batch {
    files: Vec<PathBuf>,
    next: usize,
    good: usize,
    errors: Vec<String>,
    folder: PathBuf,
    extension: String,
    cancel: Arc<AtomicBool>,
    edits: Vec<ImageEdit>,
}

/// Work that runs a step at a time between other jobs.
enum Long {
    Batch(Batch),
    Print(crate::printing::Printing, PathBuf, Edits),
}

struct Worker {
    engine: Option<crate::pdf::PdfEngine>,
    decoded: Decoded,
    com: bool,
    long: Option<Long>,
}

impl Worker {
    fn pdf(&mut self) -> std::result::Result<&mut crate::pdf::PdfEngine, String> {
        if self.engine.is_none() {
            self.engine = crate::pdf::PdfEngine::new().ok();
        }
        self.engine.as_mut().ok_or_else(|| "PDF support could not load.".into())
    }

    /// Decodes at display size, from the pre-decode cache when it has the
    /// file at this size and the file has no edits.
    fn decode(&mut self, path: &Path, width: u32, height: u32, edits: &[ImageEdit]) -> (std::result::Result<Frame, String>, bool) {
        let key = (path.to_path_buf(), width, height);
        if edits.is_empty() {
            if let Some(frame) = self.decoded.get(&key) {
                return (Ok(frame.clone()), true);
            }
        }
        let result = crate::imaging::decode_edited(path, width, height, edits);
        if let (Ok(frame), true) = (&result, edits.is_empty()) {
            self.keep(key, frame.clone());
        }
        (result, false)
    }

    fn keep(&mut self, key: (PathBuf, u32, u32), frame: Frame) {
        let bytes = frame.pixels.len();
        self.decoded.set_budget(3 * (key.1 as usize * key.2 as usize * 4).max(bytes));
        self.decoded.insert(key, frame, bytes);
    }

    fn queued(&mut self, item: &Item) -> Outcome {
        let page = match item.key.work {
            Work::Predecode { delta, width, height } => {
                if let Ok(path) = sibling(&item.path, delta) {
                    let key = (path, width, height);
                    if self.com && self.decoded.peek(&key).is_none() {
                        if let Ok(frame) = crate::imaging::decode(&key.0, width, height) {
                            self.keep(key, frame);
                        }
                    }
                }
                return Outcome::Predecoded;
            }
            Work::Tile { page, .. } | Work::Thumb { page } | Work::Notes { page } => page,
            Work::Outline => 0,
        };
        let engine = match self.pdf() {
            Ok(engine) => engine,
            Err(error) => return Outcome::Frame(Err(error)),
        };
        let (path, edits) = (&item.path, &item.edits[..]);
        match item.key.work {
            Work::Tile { .. } => Outcome::Frame(engine.render_region(path, page, item.scale, item.region, edits)),
            Work::Thumb { .. } => Outcome::Frame(engine.render_edited(path, page, item.region[2], item.region[3], edits)),
            Work::Outline => Outcome::Outline(engine.outline(path, edits)),
            _ => Outcome::Notes(engine.annotations(path, page, edits).map(|list| {
                list.into_iter()
                    .filter(|a| !matches!(a.kind.as_str(), "Link" | "Widget" | "Popup") && !a.contents.trim().is_empty())
                    .map(|a| Note { page, kind: a.kind, text: a.contents.split_whitespace().collect::<Vec<_>>().join(" ") })
                    .collect()
            })),
        }
    }

    /// Runs one job. Returns false when the window has gone.
    fn run(&mut self, job: Job, sender: &Events) -> bool {
        let job = match job {
            Job::Wake => return true,
            Job::Password(request, password) => {
                let result = self.pdf().and_then(|engine| engine.set_password(&request.path, password));
                if let Err(error) = result {
                    let opened = Opened {
                        generation: request.generation,
                        path: request.path,
                        page: request.page,
                        edits: Arc::default(),
                        result: Err(error),
                    };
                    return sender.send(Event::Pages(opened)).is_ok();
                }
                Job::Open(request)
            }
            other => other,
        };
        let event = match job {
            Job::Render(mut request) => {
                let mut from_predecode = false;
                let result = if !self.com {
                    Err("Cannot initialize Windows imaging.".into())
                } else {
                    let resolved =
                        if request.delta != 0 { sibling(&request.path, request.delta) } else { Ok(request.path.clone()) };
                    resolved.and_then(|path| {
                        request.path = path;
                        let edits = request.sessions.get(&request.path).map(|e| e.image.clone()).unwrap_or_default();
                        let (result, cached) = self.decode(&request.path, request.width, request.height, &edits);
                        from_predecode = cached;
                        result
                    })
                };
                Event::Render(Completed {
                    generation: request.generation,
                    path: request.path,
                    page: request.page,
                    navigation: request.delta != 0,
                    from_predecode,
                    result,
                })
            }
            Job::Open(request) => {
                let edits = Arc::new(request.sessions.get(&request.path).map(|e| e.pdf.clone()).unwrap_or_default());
                let result = self.pdf().map_err(|_| {
                    "PDFium could not load. Install the verified pdfium.dll beside Preview and reopen the PDF.".to_string()
                });
                let result = result.and_then(|engine| engine.page_sizes(&request.path, &edits)).and_then(|sizes| {
                    if sizes.is_empty() {
                        Err("This PDF has no pages.".into())
                    } else {
                        Ok(sizes)
                    }
                });
                Event::Pages(Opened { generation: request.generation, path: request.path, page: request.page, edits, result })
            }
            Job::Metadata(request) => {
                let edits = request.sessions.get(&request.path).map(|e| e.pdf.clone()).unwrap_or_default();
                Event::Metadata(request.generation, self.pdf().and_then(|engine| engine.metadata(&request.path, &edits)))
            }
            Job::Wake | Job::Password(..) => return true,
            job => return self.other(job, sender),
        };
        sender.send(event).is_ok()
    }

    /// Runs one file of a batch or one page of a print job. Returns false
    /// when the window has gone.
    fn step(&mut self, sender: &Events) -> bool {
        let event = match self.long.take() {
            None => return true,
            Some(Long::Batch(mut batch)) => {
                if batch.next >= batch.files.len() || batch.cancel.load(Ordering::Acquire) {
                    let total = batch.files.len();
                    let failed = batch.errors.len();
                    Event::Finished(Ok(format!(
                        "Batch finished: {} saved, {failed} failed, {} not processed.{}",
                        batch.good,
                        total - batch.good - failed,
                        batch.errors.first().map(|e| format!(" First error: {e}")).unwrap_or_default()
                    )))
                } else {
                    let path = batch.files[batch.next].clone();
                    batch.next += 1;
                    let progress = format!(
                        "Converting image {} of {}. Escape cancels after the current file.",
                        batch.next,
                        batch.files.len()
                    );
                    if sender.send(Event::Progress(progress)).is_err() {
                        return false;
                    }
                    let mut output = batch.folder.join(path.file_name().unwrap_or_default());
                    output.set_extension(&batch.extension);
                    let result = if batch.extension == "pdf" {
                        self.pdf().and_then(|engine| {
                            crate::imaging::decode_edited(&path, u32::MAX, u32::MAX, &batch.edits)
                                .and_then(|frame| engine.create_from_image(&frame, &output))
                        })
                    } else {
                        crate::imaging::export(&path, &output, &batch.edits)
                    };
                    match result {
                        Ok(()) => batch.good += 1,
                        Err(error) => {
                            batch.errors.push(format!("{}: {error}", path.file_name().unwrap_or_default().to_string_lossy()))
                        }
                    }
                    self.long = Some(Long::Batch(batch));
                    return true;
                }
            }
            Some(Long::Print(mut printing, path, edits)) => {
                let result = self.pdf().and_then(|engine| printing.page(engine, &path, &edits.pdf, &edits.image));
                match result {
                    Ok(true) => {
                        self.long = Some(Long::Print(printing, path, edits));
                        return true;
                    }
                    Ok(false) => Event::Finished(printing.finish().map(|_| "Print job sent to Windows.".into())),
                    // Dropping the job aborts the document.
                    Err(error) => Event::Finished(Err(error)),
                }
            }
        };
        sender.send(event).is_ok()
    }

    /// Saves, text, search, forms, printing, and batches.
    fn other(&mut self, job: Job, sender: &Events) -> bool {
        let request = match &job {
            Job::Save(r, _, _)
            | Job::Text(r)
            | Job::Find(r, _)
            | Job::Fields(r)
            | Job::Background(r, _)
            | Job::Print(r, _)
            | Job::Batch(r, _, _, _, _) => r.clone(),
            _ => return true,
        };
        let edits = request.sessions.get(&request.path).cloned().unwrap_or_default();
        if is_pdf(&request.path) && self.engine.is_none() {
            self.engine = crate::pdf::PdfEngine::new().ok();
        }
        let event = match job {
            Job::Save(_, output, kind) => {
                let result = if is_pdf(&request.path) {
                    match self.engine.as_mut() {
                        None => Err("PDF support could not load.".into()),
                        Some(engine) => match &kind {
                            SaveKind::ExtractPage => engine.extract_page(&request.path, &output, request.page, &edits.pdf),
                            SaveKind::Merge(other) => engine.merge(&request.path, other, &output, &edits.pdf),
                            SaveKind::Copy => engine.save_copy(&request.path, &output, &edits.pdf),
                        },
                    }
                } else if is_pdf(&output) {
                    self.pdf().and_then(|engine| {
                        crate::imaging::decode_edited(&request.path, u32::MAX, u32::MAX, &edits.image)
                            .and_then(|frame| engine.create_from_image(&frame, &output))
                    })
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
                    match self.engine.as_mut() {
                        Some(e) => e.page_text_edited(&request.path, request.page, &edits.pdf),
                        None => Err("Open a PDF first.".into()),
                    }
                } else {
                    crate::ocr::recognize(&request.path, &edits.image)
                },
            ),
            Job::Find(_, query) => Event::Found(
                request.generation,
                match self.engine.as_mut() {
                    Some(e) => e.find_edited(&request.path, &query, request.page.saturating_add(1), &edits.pdf),
                    None => Err("Open a PDF first.".into()),
                },
            ),
            Job::Fields(_) => Event::Fields(
                request.generation,
                match self.engine.as_mut() {
                    Some(e) => e.form_fields(&request.path, request.page, &edits.pdf),
                    None => Err("Open a PDF first.".into()),
                },
            ),
            Job::Background(_, output) => {
                let result = crate::background::remove(&request.path, &output, &edits.image);
                Event::Background(output, result)
            }
            Job::Print(..) | Job::Batch(..) if self.long.is_some() => {
                Event::Finished(Err("Wait for the current print or conversion to finish, then try again.".into()))
            }
            Job::Print(_, job) => match crate::printing::start(job, &request.path, is_pdf(&request.path)) {
                Ok(printing) => {
                    self.long = Some(Long::Print(printing, request.path, edits));
                    return true;
                }
                Err(error) => Event::Finished(Err(error)),
            },
            Job::Batch(_, folder, extension, cancel, selected) => {
                let files = match selected {
                    Some(selected) => Ok(selected),
                    None => std::fs::read_dir(request.path.parent().unwrap_or(Path::new("."))).map(|entries| {
                        entries
                            .filter_map(|e| e.ok())
                            .map(|e| e.path())
                            .filter(|p| {
                                p.is_file()
                                    && p.extension().is_some_and(|e| {
                                        matches!(
                                            e.to_string_lossy().to_lowercase().as_str(),
                                            "png" | "jpg" | "jpeg" | "bmp" | "tif" | "tiff" | "gif" | "webp" | "heic" | "heif"
                                        )
                                    })
                            })
                            .collect::<Vec<_>>()
                    }),
                };
                match files {
                    Err(error) => Event::Finished(Err(format!("Cannot read this folder: {error}"))),
                    Ok(mut files) => {
                        files.sort();
                        let batch = Batch { files, next: 0, good: 0, errors: Vec::new(), folder, extension, cancel, edits: edits.image };
                        self.long = Some(Long::Batch(batch));
                        return true;
                    }
                }
            }
            _ => return true,
        };
        sender.send(event).is_ok()
    }
}

fn worker(receiver: mpsc::Receiver<Job>, sender: Events, queue: Arc<Mutex<Queue>>) {
    unsafe {
        let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let mut state = Worker { engine: None, decoded: Lru::new(0), com, long: None };
        loop {
            let job = match receiver.try_recv() {
                Ok(job) => job,
                Err(mpsc::TryRecvError::Disconnected) => break,
                Err(mpsc::TryRecvError::Empty) => {
                    let item = queue.lock().ok().and_then(|mut q| q.pop());
                    if let Some(item) = item {
                        let outcome = state.queued(&item);
                        if sender.send(Event::Done(item, outcome)).is_err() {
                            break;
                        }
                        continue;
                    }
                    if state.long.is_some() {
                        if !state.step(&sender) {
                            break;
                        }
                        continue;
                    }
                    match receiver.recv() {
                        Ok(job) => job,
                        Err(_) => break,
                    }
                }
            };
            if !state.run(job, &sender) {
                break;
            }
        }
        drop(state);
        if com {
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
    queue: Arc<Mutex<Queue>>,
    pub(super) receiver: mpsc::Receiver<Event>,
    #[cfg(test)]
    thread: Option<std::thread::JoinHandle<()>>,
}

fn spawn(name: &str, events: Events, queue: Arc<Mutex<Queue>>) -> std::io::Result<(mpsc::Sender<Job>, std::thread::JoinHandle<()>)> {
    let (send, receive) = mpsc::channel();
    let thread = std::thread::Builder::new().name(name.into()).spawn(move || worker(receive, events, queue))?;
    Ok((send, thread))
}

impl Workers {
    pub(super) fn start(window: HWND) -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let events = Events { sender, window: window.0 as isize };
        let queue = Arc::default();
        let (document, _thread) = spawn("document-render", events.clone(), Arc::clone(&queue))?;
        Ok(Self {
            document,
            task: None,
            events,
            queue,
            receiver,
            #[cfg(test)]
            thread: Some(_thread),
        })
    }
    /// Returns false when the worker has stopped.
    pub(super) fn send(&mut self, job: Job) -> bool {
        if !runs_on_task_worker(&job) {
            return self.document.send(job).is_ok();
        }
        if self.task.is_none() {
            // Started on first use, so launch pays nothing for it.
            self.task = spawn("task", self.events.clone(), Arc::default()).ok().map(|(sender, _)| sender);
        }
        self.task.as_ref().is_some_and(|task| task.send(job).is_ok())
    }
    /// Replaces the document worker's queued work, most urgent first.
    pub(super) fn request(&self, items: Vec<Item>) {
        let wake = self.queue.lock().is_ok_and(|mut q| q.set(items));
        if wake {
            let _ = self.document.send(Job::Wake);
        }
    }
    pub(super) fn delivered(&self, key: &Key) {
        if let Ok(mut q) = self.queue.lock() {
            q.delivered(key);
        }
    }
    /// Stops the document worker and waits, so the next test can load PDFium.
    #[cfg(test)]
    pub(super) fn stop(mut self) {
        let thread = self.thread.take();
        drop(self);
        if let Some(thread) = thread {
            let _ = thread.join();
        }
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
        assert!(!runs_on_task_worker(&Job::Open(request("a.pdf"))));
        assert!(!runs_on_task_worker(&Job::Find(request("a.pdf"), "x".into())));
        assert!(!runs_on_task_worker(&Job::Fields(request("a.pdf"))));
    }

    fn item(page: u32) -> Item {
        Item {
            key: Key { doc: 1, work: Work::Thumb { page } },
            path: PathBuf::from("a.pdf"),
            edits: Arc::default(),
            scale: 1.0,
            region: [0, 0, 1, 1],
        }
    }

    #[test]
    fn queue_runs_the_newest_list_first_and_drops_pages_that_left() {
        let mut q = Queue::default();
        assert!(q.set(vec![item(1), item(2), item(3)]), "an idle worker needs a wake-up");
        assert_eq!(q.pop().unwrap().key, item(1).key);
        assert!(!q.set(vec![item(7), item(8)]), "the worker is busy and will pop again");
        assert_eq!(q.pop().unwrap().key, item(7).key, "the newest request runs first");
        assert_eq!(q.pop().unwrap().key, item(8).key);
        assert!(q.pop().is_none(), "pages 2 and 3 scrolled away and were dropped");
        assert!(!q.set(Vec::new()));
    }

    #[test]
    fn queue_never_repeats_work_in_flight_until_it_is_delivered() {
        let mut q = Queue::default();
        q.set(vec![item(1), item(2)]);
        let first = q.pop().unwrap();
        q.set(vec![item(1), item(2)]);
        assert_eq!(q.pop().unwrap().key, item(2).key, "page 1 is still rendering");
        q.delivered(&first.key);
        q.set(vec![item(1)]);
        assert_eq!(q.pop().unwrap().key, item(1).key, "asked again after delivery");
    }

    /// A batch of 30 images into PDFs runs on the document worker. A PDF
    /// opened right after it must not wait for the whole batch.
    #[test]
    fn batches_yield_to_the_view_between_files() {
        let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
        if !fixtures.join("image-small.png").is_file() {
            eprintln!("skipped: fixtures are missing; run tools/make-fixtures.ps1");
            return;
        }
        let folder = std::env::temp_dir().join(format!("pfw-batch-{}", std::process::id()));
        let out = folder.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let files: Vec<PathBuf> = (0..30)
            .map(|n| {
                let file = folder.join(format!("{n:02}.png"));
                std::fs::copy(fixtures.join("image-small.png"), &file).unwrap();
                file
            })
            .collect();
        let pdf = std::fs::canonicalize(fixtures.join("20-pages.pdf")).unwrap();
        let mut workers = Workers::start(HWND::default()).unwrap();
        let request = |path: &Path| Request {
            generation: 1,
            path: path.to_path_buf(),
            page: 0,
            delta: 0,
            width: 1,
            height: 1,
            sessions: HashMap::new(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        assert!(workers.send(Job::Batch(request(&files[0]), out.clone(), "pdf".into(), cancel, Some(files.clone()))));
        let mut order = Vec::new();
        loop {
            match workers.receiver.recv_timeout(std::time::Duration::from_secs(60)).expect("the worker stopped") {
                // Open the PDF once the batch is under way.
                Event::Progress(_) if order.is_empty() => {
                    assert!(workers.send(Job::Open(request(&pdf))));
                    order.push("started");
                }
                Event::Pages(opened) => {
                    assert_eq!(opened.result.unwrap().len(), 20);
                    order.push("pages");
                }
                Event::Finished(result) => {
                    assert!(result.unwrap().starts_with("Batch finished: 30 saved"));
                    order.push("finished");
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(order, vec!["started", "pages", "finished"], "the PDF opened before the batch ended");
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), 30);
        workers.stop();
        std::fs::remove_dir_all(folder).unwrap();
    }

    /// Opens the 500-page fixture on a real document worker, renders tiles
    /// through the queue, and prints timings. Headless: no window.
    #[test]
    fn document_worker_renders_tiles_and_thumbnails_headless() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/500-pages-50mb.pdf");
        if !path.is_file() {
            eprintln!("skipped: fixtures are missing; run tools/make-fixtures.ps1");
            return;
        }
        let path = std::fs::canonicalize(path).unwrap();
        let mut workers = Workers::start(HWND::default()).unwrap();
        let timeout = std::time::Duration::from_secs(30);
        let started = std::time::Instant::now();
        let request = Request { generation: 7, path: path.clone(), page: 0, delta: 0, width: 1, height: 1, sessions: HashMap::new() };
        assert!(workers.send(Job::Open(request)));
        let Ok(Event::Pages(opened)) = workers.receiver.recv_timeout(timeout) else { panic!("no page sizes") };
        let sizes = opened.result.unwrap();
        let open_ms = started.elapsed().as_secs_f64() * 1000.0;
        assert_eq!((opened.generation, sizes.len()), (7, 500));
        let doc = doc_id(&path);
        let scale = 1.5f32;
        let px = super::super::view::page_px(sizes[0], scale);
        let tiles = super::super::view::tiles(px, super::super::widgets::Rect::new(0.0, 0.0, 1e6, 1e6));
        let tile = |page: u32, (col, row, region): (u32, u32, [u32; 4])| Item {
            key: Key { doc, work: Work::Tile { page, scale: scale.to_bits(), col, row } },
            path: path.clone(),
            edits: Arc::clone(&opened.edits),
            scale,
            region,
        };
        let started = std::time::Instant::now();
        workers.request(tiles.iter().map(|t| tile(0, *t)).collect());
        let mut first_ms = None;
        for _ in 0..tiles.len() {
            let Ok(Event::Done(item, Outcome::Frame(frame))) = workers.receiver.recv_timeout(timeout) else { panic!("no tile") };
            workers.delivered(&item.key);
            let frame = frame.unwrap();
            assert_eq!((frame.width, frame.height), (item.region[2], item.region[3]));
            first_ms.get_or_insert(started.elapsed().as_secs_f64() * 1000.0);
        }
        let page_ms = started.elapsed().as_secs_f64() * 1000.0;
        // Newest first: page 400 replaces page 300 before the worker gets to it.
        workers.request((0..2).map(|i| tile(300, tiles[i])).collect());
        workers.request(vec![tile(400, tiles[0])]);
        let mut pages = Vec::new();
        while let Ok(Event::Done(item, _)) = workers.receiver.recv_timeout(std::time::Duration::from_millis(500)) {
            workers.delivered(&item.key);
            if let Work::Tile { page, .. } = item.key.work {
                pages.push(page);
            }
        }
        assert!(pages.contains(&400) && pages.iter().filter(|p| **p == 300).count() <= 1, "{pages:?}");
        let started = std::time::Instant::now();
        let thumbs: Vec<Item> = (0..20)
            .map(|page| Item { key: Key { doc, work: Work::Thumb { page } }, region: [0, 0, 120, 160], ..tile(0, tiles[0]) })
            .collect();
        workers.request(thumbs);
        for _ in 0..20 {
            let Ok(Event::Done(item, Outcome::Frame(frame))) = workers.receiver.recv_timeout(timeout) else { panic!("no thumbnail") };
            workers.delivered(&item.key);
            assert!(frame.unwrap().width <= 120);
        }
        let thumb_ms = started.elapsed().as_secs_f64() * 1000.0 / 20.0;
        println!(
            "500-page PDF: open and page sizes {open_ms:.1} ms; first 512 px tile {:.1} ms; all {} tiles of page 1 at 150% {page_ms:.1} ms; thumbnail {thumb_ms:.2} ms each",
            first_ms.unwrap(),
            tiles.len()
        );
        workers.stop();
    }
}
