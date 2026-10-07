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
use super::{cache::Lru, disk, text::{Pos, Selection, END}};
use crate::model::{Frame, ImageEdit, OutlineItem, PdfEdit, PdfMetadata, SearchHit, TextLayer};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    os::windows::io::AsRawHandle,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
};
use windows::Win32::{
    Foundation::{HANDLE, HWND, LPARAM, WPARAM},
    Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
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
    /// Extracts the page to a temporary file to drag out to Explorer.
    DragOut,
}

#[derive(Clone, Default, Debug)]
pub(super) struct Edits {
    pub(super) image: Vec<ImageEdit>,
    pub(super) pdf: Vec<PdfEdit>,
    pub(super) dirty: bool,
}

#[derive(Clone, Debug)]
pub(super) struct AutosaveRequest {
    pub(super) path: PathBuf,
    pub(super) session_id: u64,
    pub(super) revision: u64,
    pub(super) opened: disk::Stamp,
    pub(super) snapshot: Option<PathBuf>,
    pub(super) snapshot_dir: PathBuf,
    pub(super) target: PathBuf,
    pub(super) expected: Option<disk::Stamp>,
    pub(super) force: bool,
    pub(super) edits: Edits,
}

#[derive(Debug)]
pub(super) struct Autosaved {
    pub(super) path: PathBuf,
    pub(super) session_id: u64,
    pub(super) revision: u64,
    /// Present once the immutable opened copy was made, including when the
    /// later target write failed. A retry must reuse it.
    pub(super) snapshot: Option<PathBuf>,
    pub(super) target: PathBuf,
    pub(super) result: Result<disk::Stamp, disk::Failure>,
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
    pub(super) sources: HashMap<PathBuf, PathBuf>,
}
impl Request {
    fn source(&self) -> &Path {
        self.sources.get(&self.path).map(PathBuf::as_path).unwrap_or(&self.path)
    }
}

type ReaderCounts = Arc<Mutex<HashMap<PathBuf, usize>>>;

struct QueuedJob {
    job: Job,
    readers: Vec<PathBuf>,
}

fn reader_paths(job: &Job) -> Vec<PathBuf> {
    if let Job::Render(request) = job {
        if request.delta != 0 {
            // The sibling is resolved on the worker after enqueue. Keep every
            // candidate snapshot alive until that navigation read finishes.
            return request.sources.keys().cloned().collect();
        }
    }
    let (request, selected) = match job {
        Job::Render(request)
        | Job::Open(request)
        | Job::Metadata(request)
        | Job::Save(request, _, _)
        | Job::Layer(request)
        | Job::Text(request, _)
        | Job::Find(request, _)
        | Job::Fields(request)
        | Job::Background(request, _)
        | Job::Print(request, _)
        | Job::Password(request, _) => (request, None),
        Job::Batch(request, _, _, _, selected) => (request, selected.as_ref()),
        Job::Autosave(_) | Job::Wake => return Vec::new(),
    };
    match selected {
        Some(files) => files.iter().filter(|path| request.sources.contains_key(*path)).cloned().collect(),
        None if matches!(job, Job::Batch(..)) => request.sources.keys().cloned().collect(),
        None => request.sources.contains_key(&request.path).then(|| request.path.clone()).into_iter().collect(),
    }
}

fn retain_readers(counts: &ReaderCounts, paths: &[PathBuf]) {
    if let Ok(mut counts) = counts.lock() {
        for path in paths {
            *counts.entry(path.clone()).or_default() += 1;
        }
    }
}

fn release_readers(counts: &ReaderCounts, paths: &[PathBuf]) {
    if let Ok(mut counts) = counts.lock() {
        for path in paths {
            if let Some(count) = counts.get_mut(path) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    counts.remove(path);
                }
            }
        }
    }
}
pub(super) enum Job {
    /// Decodes an image (or its neighbor when `delta` is not 0).
    Render(Request),
    /// Opens a PDF and reads its page sizes.
    Open(Request),
    Metadata(Request),
    Save(Request, PathBuf, SaveKind),
    #[allow(dead_code)] // First-edit consent schedules this in the next save-model stage.
    Autosave(AutosaveRequest),
    Layer(Request),
    Text(Request, Option<Selection>),
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
    /// The temporary file for a page drag out, or why it failed.
    DragOut(PathBuf, std::result::Result<(), String>),
    Autosaved(Autosaved),
    Layer(u64, PathBuf, u32, std::result::Result<TextLayer, String>),
    Text(u64, PathBuf, std::result::Result<String, String>),
    Found(u64, PathBuf, String, std::result::Result<Vec<SearchHit>, String>),
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
    pub(super) opened: Option<disk::Stamp>,
    pub(super) result: std::result::Result<Frame, String>,
}
/// Page sizes in points of an opened PDF, after `edits`.
pub(super) struct Opened {
    pub(super) generation: u64,
    pub(super) path: PathBuf,
    pub(super) page: u32,
    pub(super) edits: Arc<Vec<PdfEdit>>,
    pub(super) opened: Option<disk::Stamp>,
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
    /// Logical file whose immutable snapshot an item reads. Kept until the
    /// waiting item is dropped or the UI acknowledges its completion.
    leases: HashMap<Key, PathBuf>,
}
impl Queue {
    /// Replaces the waiting work. Returns true when the worker may be
    /// waiting on its channel and needs a wake-up.
    #[cfg(test)]
    pub(super) fn set(&mut self, items: Vec<Item>) -> bool {
        self.set_with_sources(items, &HashMap::new()).0
    }
    fn set_with_sources(
        &mut self,
        items: Vec<Item>,
        sources: &HashMap<PathBuf, PathBuf>,
    ) -> (bool, Vec<PathBuf>, Vec<PathBuf>) {
        let idle = self.items.is_empty();
        let taken = &self.taken;
        let next: VecDeque<Item> = items.into_iter().filter(|i| !taken.contains(&i.key)).collect();
        let desired: HashMap<Key, PathBuf> = next
            .iter()
            .filter_map(|item| {
                sources
                    .iter()
                    .find(|(_, source)| *source == &item.path)
                    .map(|(logical, _)| (item.key, logical.clone()))
            })
            .collect();
        let waiting: Vec<Key> = self.items.iter().map(|item| item.key).collect();
        let mut released = Vec::new();
        for key in waiting {
            if self.leases.get(&key) != desired.get(&key) {
                if let Some(path) = self.leases.remove(&key) {
                    released.push(path);
                }
            }
        }
        let mut retained = Vec::new();
        for (key, path) in desired {
            if self.leases.get(&key) != Some(&path) {
                self.leases.insert(key, path.clone());
                retained.push(path);
            }
        }
        self.items = next;
        (idle && !self.items.is_empty(), retained, released)
    }
    pub(super) fn pop(&mut self) -> Option<Item> {
        let item = self.items.pop_front()?;
        self.taken.insert(item.key);
        Some(item)
    }
    pub(super) fn delivered(&mut self, key: &Key) -> Option<PathBuf> {
        self.taken.remove(key);
        self.leases.remove(key)
    }
}

pub(super) fn is_pdf(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

fn same_image_format(source: &Path, target: &Path) -> bool {
    let family = |path: &Path| match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" | "jpe" | "jfif" => Some(0),
        "png" => Some(1),
        "webp" => Some(2),
        "tif" | "tiff" => Some(3),
        "heic" | "heif" => Some(4),
        "bmp" => Some(5),
        _ => None,
    };
    family(source).is_some() && family(source) == family(target)
}

/// Reads a document only when its size and timestamps stay unchanged for the
/// whole operation. The returned stamp identifies the bytes shown to the user.
fn stable_read<T>(path: &Path, read: impl FnOnce() -> Result<T, String>) -> Result<(T, disk::Stamp), String> {
    let stamp = disk::Stamp::of(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    let value = read()?;
    let after = disk::Stamp::of(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    if stamp != after {
        return Err("The file changed while Preview was opening it. Open it again.".into());
    }
    Ok((value, stamp))
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
type Decoded = Lru<(PathBuf, u32, u32), (ImageRevision, Frame)>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct ImageRevision {
    length: u64,
    last_write_time: u64,
    volume_serial_number: u32,
    file_index: u64,
}
impl ImageRevision {
    fn read(path: &Path) -> Option<Self> {
        let file = std::fs::File::open(path).ok()?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info).ok()? };
        Some(Self {
            length: ((info.nFileSizeHigh as u64) << 32) | info.nFileSizeLow as u64,
            last_write_time: ((info.ftLastWriteTime.dwHighDateTime as u64) << 32)
                | info.ftLastWriteTime.dwLowDateTime as u64,
            volume_serial_number: info.dwVolumeSerialNumber,
            file_index: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
        })
    }
}

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
    sources: HashMap<PathBuf, PathBuf>,
    readers: Vec<PathBuf>,
}

/// Work that runs a step at a time between other jobs.
enum Long {
    Batch(Batch),
    Print(crate::printing::Printing, PathBuf, Edits, Vec<PathBuf>),
}

struct Worker {
    engine: Option<crate::pdf::PdfEngine>,
    decoded: Decoded,
    com: bool,
    long: Option<Long>,
    readers: ReaderCounts,
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
        let revision = edits.is_empty().then(|| ImageRevision::read(path)).flatten();
        if let Some(current) = &revision {
            if let Some((cached, frame)) = self.decoded.get(&key) {
                if cached == current {
                    return (Ok(frame.clone()), true);
                }
            }
        }
        let result = crate::imaging::decode_edited(path, width, height, edits);
        if let (Ok(frame), Some(revision)) = (&result, revision) {
            // A replacement during decoding must not label old pixels with a new revision.
            if ImageRevision::read(path).as_ref() == Some(&revision) {
                self.keep(key, revision, frame.clone());
            }
        }
        (result, false)
    }

    fn keep(&mut self, key: (PathBuf, u32, u32), revision: ImageRevision, frame: Frame) {
        let bytes = frame.pixels.len();
        self.decoded.set_budget(3 * (key.1 as usize * key.2 as usize * 4).max(bytes));
        self.decoded.insert(key, (revision, frame), bytes);
    }

    /// Rebuilds one revision from the immutable bytes that were opened, then
    /// atomically publishes it only if the destination is still the version
    /// the window last observed.
    fn autosave(&mut self, request: AutosaveRequest) -> Autosaved {
        let snapshot = match request.snapshot {
            Some(path) => Ok(path),
            None => disk::snapshot(&request.path, &request.snapshot_dir, request.opened),
        };
        let (snapshot, result) = match snapshot {
            Err(error) => (None, Err(error)),
            Ok(snapshot) => {
                let result = disk::write_verified(&request.target, request.expected, request.force, |staged| {
                    if is_pdf(&request.path) {
                        self.pdf()?
                            .save_incremental(&snapshot, &request.edits.pdf, staged)
                            .map(|_| ())
                    } else if request.edits.image.is_empty() && same_image_format(&request.path, &request.target) {
                        disk::copy_verified(&snapshot, staged)
                    } else {
                        crate::imaging::export(&snapshot, staged, &request.edits.image)
                    }
                });
                (Some(snapshot), result)
            }
        };
        Autosaved {
            path: request.path,
            session_id: request.session_id,
            revision: request.revision,
            snapshot,
            target: request.target,
            result,
        }
    }

    fn queued(&mut self, item: &Item) -> Outcome {
        let page = match item.key.work {
            Work::Predecode { delta, width, height } => {
                if let Ok(path) = sibling(&item.path, delta) {
                    if self.com {
                        let _ = self.decode(&path, width, height, &[]);
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
    fn run(&mut self, job: Job, readers: Vec<PathBuf>, sender: &Events) -> bool {
        let job = match job {
            Job::Wake => {
                release_readers(&self.readers, &readers);
                return true;
            }
            Job::Password(request, password) => {
                let result = self.pdf().and_then(|engine| engine.set_password(&request.path, password));
                if let Err(error) = result {
                    let opened = Opened {
                        generation: request.generation,
                        path: request.path,
                        page: request.page,
                        edits: Arc::default(),
                        opened: None,
                        result: Err(error),
                    };
                    let alive = sender.send(Event::Pages(opened)).is_ok();
                    release_readers(&self.readers, &readers);
                    return alive;
                }
                Job::Open(request)
            }
            other => other,
        };
        let event = match job {
            Job::Render(mut request) => {
                let mut from_predecode = false;
                let mut opened = None;
                let result = if !self.com {
                    Err("Cannot initialize Windows imaging.".into())
                } else {
                    let resolved =
                        if request.delta != 0 { sibling(&request.path, request.delta) } else { Ok(request.path.clone()) };
                    resolved.and_then(|path| {
                        request.path = path;
                        let edits = request.sessions.get(&request.path).map(|e| e.image.clone()).unwrap_or_default();
                        let source = request.source().to_path_buf();
                        stable_read(&source, || {
                            let (result, cached) = self.decode(&source, request.width, request.height, &edits);
                            from_predecode = cached;
                            result
                        })
                        .map(|(frame, stamp)| {
                            opened = Some(stamp);
                            frame
                        })
                    })
                };
                Event::Render(Completed {
                    generation: request.generation,
                    path: request.path,
                    page: request.page,
                    navigation: request.delta != 0,
                    from_predecode,
                    opened,
                    result,
                })
            }
            Job::Open(request) => {
                let edits = Arc::new(request.sessions.get(&request.path).map(|e| e.pdf.clone()).unwrap_or_default());
                let source = request.source().to_path_buf();
                let result = stable_read(&source, || {
                    self.pdf()
                        .map_err(|_| {
                            "PDFium could not load. Install the verified pdfium.dll beside Preview and reopen the PDF.".to_string()
                        })
                        .and_then(|engine| {
                            engine.alias_password(&request.path, &source);
                            engine.page_sizes(&source, &edits)
                        })
                        .and_then(|sizes| {
                            if sizes.is_empty() {
                                Err("This PDF has no pages.".into())
                            } else {
                                Ok(sizes)
                            }
                        })
                });
                let (opened, result) = match result {
                    Ok((sizes, stamp)) => (Some(stamp), Ok(sizes)),
                    Err(error) => (None, Err(error)),
                };
                Event::Pages(Opened {
                    generation: request.generation,
                    path: request.path,
                    page: request.page,
                    edits,
                    opened,
                    result,
                })
            }
            Job::Metadata(request) => {
                let edits = request.sessions.get(&request.path).map(|e| e.pdf.clone()).unwrap_or_default();
                let source = request.source().to_path_buf();
                Event::Metadata(request.generation, self.pdf().and_then(|engine| {
                    engine.alias_password(&request.path, &source);
                    engine.metadata(&source, &edits)
                }))
            }
            Job::Autosave(request) => Event::Autosaved(self.autosave(request)),
            Job::Wake | Job::Password(..) => return true,
            job => return self.other(job, readers, sender),
        };
        let alive = sender.send(event).is_ok();
        release_readers(&self.readers, &readers);
        alive
    }

    /// Runs one file of a batch or one page of a print job. Returns false
    /// when the window has gone.
    fn step(&mut self, sender: &Events) -> bool {
        let completed_readers;
        let event = match self.long.take() {
            None => return true,
            Some(Long::Batch(mut batch)) => {
                if batch.next >= batch.files.len() || batch.cancel.load(Ordering::Acquire) {
                    completed_readers = std::mem::take(&mut batch.readers);
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
                    let source = batch.sources.get(&path).unwrap_or(&path).clone();
                    let edits = batch.edits.as_slice();
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
                            crate::imaging::decode_edited(&source, u32::MAX, u32::MAX, edits)
                                .and_then(|frame| engine.create_from_image(&frame, &output))
                        })
                    } else {
                        crate::imaging::export(&source, &output, edits)
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
            Some(Long::Print(mut printing, path, edits, readers)) => {
                let result = self.pdf().and_then(|engine| printing.page(engine, &path, &edits.pdf, &edits.image));
                match result {
                    Ok(true) => {
                        self.long = Some(Long::Print(printing, path, edits, readers));
                        return true;
                    }
                    Ok(false) => {
                        completed_readers = readers;
                        Event::Finished(printing.finish().map(|_| "Print job sent to Windows.".into()))
                    }
                    // Dropping the job aborts the document.
                    Err(error) => {
                        completed_readers = readers;
                        Event::Finished(Err(error))
                    }
                }
            }
        };
        let alive = sender.send(event).is_ok();
        release_readers(&self.readers, &completed_readers);
        alive
    }

    /// Saves, text, search, forms, printing, and batches.
    fn other(&mut self, job: Job, readers: Vec<PathBuf>, sender: &Events) -> bool {
        let request = match &job {
            Job::Save(r, _, _)
            | Job::Layer(r)
            | Job::Text(r, _)
            | Job::Find(r, _)
            | Job::Fields(r)
            | Job::Background(r, _)
            | Job::Print(r, _)
            | Job::Batch(r, _, _, _, _) => r.clone(),
            _ => return true,
        };
        let edits = request.sessions.get(&request.path).cloned().unwrap_or_default();
        let source = request.source().to_path_buf();
        if is_pdf(&request.path) && self.engine.is_none() {
            self.engine = crate::pdf::PdfEngine::new().ok();
        }
        let event = match job {
            Job::Save(_, output, kind) => {
                let result = if is_pdf(&request.path) {
                    match self.engine.as_mut() {
                        None => Err("PDF support could not load.".into()),
                        Some(engine) => {
                            engine.alias_password(&request.path, &source);
                            match &kind {
                                SaveKind::ExtractPage | SaveKind::DragOut => {
                                    engine.extract_page(&source, &output, request.page, &edits.pdf)
                                }
                                SaveKind::Merge(other) => engine.merge(&source, other, &output, &edits.pdf),
                                SaveKind::Copy => engine.save_copy(&source, &output, &edits.pdf),
                            }
                        }
                    }
                } else if is_pdf(&output) {
                    self.pdf().and_then(|engine| {
                        crate::imaging::decode_edited(&source, u32::MAX, u32::MAX, &edits.image)
                            .and_then(|frame| engine.create_from_image(&frame, &output))
                    })
                } else {
                    crate::imaging::export(&source, &output, &edits.image)
                };
                if kind == SaveKind::DragOut {
                    Event::DragOut(output, result.map_err(|e| format!("Could not drag this page out: {e}")))
                } else {
                    Event::Saved(
                        request.path,
                        edits,
                        kind != SaveKind::ExtractPage,
                        result.map_err(|e| format!("Could not save {}: {e}", output.display())),
                    )
                }
            }
            Job::Layer(_) => Event::Layer(
                request.generation,
                request.path.clone(),
                request.page,
                if is_pdf(&request.path) {
                    match self.engine.as_mut() {
                        Some(e) => {
                            e.alias_password(&request.path, &source);
                            e.text_layer(&source, request.page, &edits.pdf)
                        }
                        None => Err("Open a PDF first.".into()),
                    }
                } else {
                    crate::ocr::recognize_layer(&source, &edits.image)
                },
            ),
            Job::Text(_, selection) => Event::Text(
                request.generation,
                request.path.clone(),
                if is_pdf(&request.path) {
                    match self.engine.as_mut() {
                        Some(e) => {
                            e.alias_password(&request.path, &source);
                            match e.can_copy(&source, &edits.pdf) {
                                Ok(true) => match selection {
                                    Some(selection) => super::text::selected_text(&selection, |page| e.text_layer(&source, page, &edits.pdf)),
                                    None => {
                                        let page = request.page;
                                        let selection = Selection { anchor: Pos { page, index: 0 }, focus: Pos { page, index: END } };
                                        super::text::selected_text(&selection, |page| e.text_layer(&source, page, &edits.pdf))
                                    }
                                },
                                Ok(false) => Err("This PDF's permissions do not allow copying text.".into()),
                                Err(error) => Err(error),
                            }
                        }
                        None => Err("Open a PDF first.".into()),
                    }
                } else {
                    match selection {
                        Some(selection) => crate::ocr::recognize_layer(&source, &edits.image)
                            .and_then(|layer| super::text::selected_text(&selection, |_| Ok(layer.clone()))),
                        None => crate::ocr::recognize(&source, &edits.image),
                    }
                },
            ),
            Job::Find(_, query) => Event::Found(
                request.generation,
                request.path.clone(),
                query.clone(),
                if is_pdf(&request.path) {
                    match self.engine.as_mut() {
                        Some(e) => {
                            e.alias_password(&request.path, &source);
                            e.search(&source, &query, false, &AtomicBool::new(false), &edits.pdf)
                        }
                        None => Err("Open a PDF first.".into()),
                    }
                } else {
                    crate::ocr::recognize_layer(&source, &edits.image)
                        .map(|layer| super::text::find(&layer, 0, &query, false))
                },
            ),
            Job::Fields(_) => Event::Fields(
                request.generation,
                match self.engine.as_mut() {
                    Some(e) => {
                        e.alias_password(&request.path, &source);
                        e.form_fields(&source, request.page, &edits.pdf)
                    }
                    None => Err("Open a PDF first.".into()),
                },
            ),
            Job::Background(_, output) => {
                let result = crate::background::remove(&source, &output, &edits.image);
                Event::Background(output, result)
            }
            Job::Print(..) | Job::Batch(..) if self.long.is_some() => {
                Event::Finished(Err("Wait for the current print or conversion to finish, then try again.".into()))
            }
            Job::Print(_, job) => match crate::printing::start(job, &source, is_pdf(&request.path)) {
                Ok(printing) => {
                    self.long = Some(Long::Print(printing, source, edits, readers));
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
                        let batch = Batch {
                            files,
                            next: 0,
                            good: 0,
                            errors: Vec::new(),
                            folder,
                            extension,
                            cancel,
                            edits: edits.image,
                            sources: request.sources,
                            readers,
                        };
                        self.long = Some(Long::Batch(batch));
                        return true;
                    }
                }
            }
            _ => return true,
        };
        let alive = sender.send(event).is_ok();
        release_readers(&self.readers, &readers);
        alive
    }
}

fn worker(receiver: mpsc::Receiver<QueuedJob>, sender: Events, queue: Arc<Mutex<Queue>>, readers: ReaderCounts) {
    unsafe {
        let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let mut state = Worker { engine: None, decoded: Lru::new(0), com, long: None, readers };
        loop {
            let job = match receiver.try_recv() {
                Ok(job) => job,
                Err(mpsc::TryRecvError::Disconnected) => break,
                Err(mpsc::TryRecvError::Empty) => {
                    let item = queue.lock().ok().and_then(|mut q| q.pop());
                    if let Some(item) = item {
                        let outcome = state.queued(&item);
                        let key = item.key;
                        if sender.send(Event::Done(item, outcome)).is_err() {
                            let lease = queue.lock().ok().and_then(|mut q| q.delivered(&key));
                            if let Some(path) = lease {
                                release_readers(&state.readers, &[path]);
                            }
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
            if !state.run(job.job, job.readers, &sender) {
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
        Job::Layer(request) | Job::Text(request, _) => !is_pdf(&request.path),
        Job::Background(request, _) => !request.sources.contains_key(&request.path),
        Job::Batch(request, _, extension, _, _) => extension != "pdf" && request.sources.is_empty(),
        Job::Save(request, output, SaveKind::Copy) => {
            !is_pdf(&request.path) && !is_pdf(output) && !request.sources.contains_key(&request.path)
        }
        _ => false,
    }
}

pub(super) struct Workers {
    document: mpsc::Sender<QueuedJob>,
    task: Option<mpsc::Sender<QueuedJob>>,
    events: Events,
    queue: Arc<Mutex<Queue>>,
    readers: ReaderCounts,
    pub(super) receiver: mpsc::Receiver<Event>,
    #[cfg(test)]
    thread: Option<std::thread::JoinHandle<()>>,
}

fn spawn(
    name: &str,
    events: Events,
    queue: Arc<Mutex<Queue>>,
    readers: ReaderCounts,
) -> std::io::Result<(mpsc::Sender<QueuedJob>, std::thread::JoinHandle<()>)> {
    let (send, receive) = mpsc::channel();
    let thread = std::thread::Builder::new().name(name.into()).spawn(move || worker(receive, events, queue, readers))?;
    Ok((send, thread))
}

impl Workers {
    pub(super) fn start(window: HWND) -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let events = Events { sender, window: window.0 as isize };
        let queue = Arc::default();
        let readers = Arc::default();
        let (document, _thread) = spawn("document-render", events.clone(), Arc::clone(&queue), Arc::clone(&readers))?;
        Ok(Self {
            document,
            task: None,
            events,
            queue,
            readers,
            receiver,
            #[cfg(test)]
            thread: Some(_thread),
        })
    }
    /// Returns false when the worker has stopped.
    pub(super) fn send(&mut self, job: Job) -> bool {
        let task_worker = runs_on_task_worker(&job);
        let readers = reader_paths(&job);
        retain_readers(&self.readers, &readers);
        let job = QueuedJob { job, readers: readers.clone() };
        if !task_worker {
            let sent = self.document.send(job).is_ok();
            if !sent {
                release_readers(&self.readers, &readers);
            }
            return sent;
        }
        if self.task.is_none() {
            // Started on first use, so launch pays nothing for it.
            self.task = spawn("task", self.events.clone(), Arc::default(), Arc::clone(&self.readers)).ok().map(|(sender, _)| sender);
        }
        let sent = self.task.as_ref().is_some_and(|task| task.send(job).is_ok());
        if !sent {
            release_readers(&self.readers, &readers);
        }
        sent
    }
    pub(super) fn reads_snapshot(&self, path: &Path) -> bool {
        self.readers.lock().is_ok_and(|readers| readers.get(path).is_some_and(|count| *count > 0))
    }
    pub(super) fn reads_any_snapshot(&self) -> bool {
        self.readers.lock().is_ok_and(|readers| readers.values().any(|count| *count > 0))
    }
    #[cfg(test)]
    pub(super) fn retain_test_job_readers(&self, job: &Job) {
        retain_readers(&self.readers, &reader_paths(job));
    }
    /// Replaces the document worker's queued work, most urgent first.
    #[cfg(test)]
    pub(super) fn request(&self, items: Vec<Item>) {
        self.request_with_sources(items, &HashMap::new());
    }
    pub(super) fn request_with_sources(&self, items: Vec<Item>, sources: &HashMap<PathBuf, PathBuf>) {
        let Some((wake, retained, released)) = self.queue.lock().ok().map(|mut q| q.set_with_sources(items, sources)) else {
            return;
        };
        retain_readers(&self.readers, &retained);
        release_readers(&self.readers, &released);
        if wake {
            let _ = self.document.send(QueuedJob { job: Job::Wake, readers: Vec::new() });
        }
    }
    pub(super) fn delivered(&self, key: &Key) {
        let lease = self.queue.lock().ok().and_then(|mut q| q.delivered(key));
        if let Some(path) = lease {
            release_readers(&self.readers, &[path]);
        }
    }
    #[cfg(test)]
    pub(super) fn take_test_view_item(&self) -> Option<Item> {
        self.queue.lock().ok()?.pop()
    }
    #[cfg(test)]
    pub(super) fn retain_test_view_items(&self, items: Vec<Item>, sources: &HashMap<PathBuf, PathBuf>) {
        if let Ok(mut queue) = self.queue.lock() {
            let (_, retained, released) = queue.set_with_sources(items, sources);
            retain_readers(&self.readers, &retained);
            release_readers(&self.readers, &released);
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

    fn temp(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pfw-autosave-{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn autosave_request(
        path: PathBuf,
        target: PathBuf,
        opened: disk::Stamp,
        snapshot_dir: PathBuf,
        edits: Edits,
    ) -> AutosaveRequest {
        AutosaveRequest {
            path,
            session_id: 1,
            revision: 1,
            opened,
            snapshot: None,
            snapshot_dir,
            target,
            expected: Some(opened),
            force: false,
            edits,
        }
    }

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
    fn opening_rejects_a_file_that_changes_during_the_read() {
        let dir = temp("opening-change");
        let path = dir.join("changing.bin");
        std::fs::write(&path, b"before").unwrap();
        let result = stable_read(&path, || {
            std::fs::write(&path, b"after and a different length").unwrap();
            Ok(())
        });
        assert_eq!(result.unwrap_err(), "The file changed while Preview was opening it. Open it again.");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn image_autosave_uses_the_opened_snapshot_and_reports_external_change() {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap() };
        let dir = temp("image");
        let source = dir.join("photo.png");
        let frame = Frame {
            width: 2,
            height: 1,
            pixels: [0, 0, 255, 255, 0, 255, 0, 255].to_vec(),
            page_count: 1,
            source_width: 2,
            source_height: 1,
        };
        crate::imaging::export_frame(&frame, &source).unwrap();
        let opened = disk::Stamp::of(&source).unwrap();
        let edits = Edits { image: vec![ImageEdit::RotateRight], pdf: Vec::new(), dirty: true };
        let mut worker = Worker { engine: None, decoded: Lru::new(0), com: true, long: None, readers: Arc::default() };
        let saved = worker.autosave(autosave_request(
            source.clone(),
            source.clone(),
            opened,
            dir.join("opened"),
            edits.clone(),
        ));
        let saved_stamp = saved.result.unwrap();
        let snapshot = saved.snapshot.unwrap();
        let original = crate::imaging::decode(&snapshot, 100, 100).unwrap();
        let rotated = crate::imaging::decode(&source, 100, 100).unwrap();
        assert_eq!((original.source_width, original.source_height), (2, 1));
        assert_eq!((rotated.source_width, rotated.source_height), (1, 2));

        // Replace the target after Preview's successful save. The retry keeps
        // the immutable snapshot but must not overwrite the other writer.
        std::fs::remove_file(&source).unwrap();
        let external = Frame { width: 3, source_width: 3, pixels: [255, 0, 0, 255].repeat(3), ..frame };
        crate::imaging::export_frame(&external, &source).unwrap();
        let mut retry = autosave_request(
            source.clone(),
            source.clone(),
            opened,
            dir.join("unused"),
            edits,
        );
        retry.revision = 2;
        retry.snapshot = Some(snapshot);
        retry.expected = Some(saved_stamp);
        let changed = worker.autosave(retry);
        assert_eq!(changed.result, Err(disk::Failure::Changed));
        assert_eq!(crate::imaging::decode(&source, 100, 100).unwrap().source_width, 3);
        drop(worker);
        std::fs::remove_dir_all(dir).unwrap();
        unsafe { CoUninitialize() };
    }

    #[test]
    fn image_autosave_without_edits_preserves_jpeg_bytes() {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/image-24mp.jpg");
        assert!(fixture.is_file(), "run tools/make-fixtures.ps1 before this test");
        let dir = temp("jpeg-copy");
        let source = dir.join("photo.jpg");
        let target = dir.join("photo-copy.jpg");
        std::fs::copy(&fixture, &source).unwrap();
        let opened_bytes = std::fs::read(&source).unwrap();
        let opened = disk::Stamp::of(&source).unwrap();
        let mut worker = Worker { engine: None, decoded: Lru::new(0), com: false, long: None, readers: Arc::default() };
        let mut request = autosave_request(
            source,
            target.clone(),
            opened,
            dir.join("opened"),
            Edits::default(),
        );
        request.expected = None;
        let saved = worker.autosave(request);
        saved.result.unwrap();
        assert_eq!(
            std::fs::read(&target).unwrap(),
            opened_bytes,
            "a no-op save keeps the exact JPEG stream, including metadata"
        );
        drop(worker);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn image_autosave_without_edits_converts_jpeg_to_png() {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap() };
        let dir = temp("jpeg-to-png");
        let source = dir.join("photo.jpg");
        let target = dir.join("photo.png");
        let frame = Frame {
            width: 2,
            height: 1,
            pixels: vec![0, 0, 255, 255, 0, 255, 0, 255],
            page_count: 1,
            source_width: 2,
            source_height: 1,
        };
        crate::imaging::export_frame(&frame, &source).unwrap();
        let jpeg = std::fs::read(&source).unwrap();
        assert!(jpeg.starts_with(&[0xff, 0xd8]), "the source fixture is JPEG");
        let opened = disk::Stamp::of(&source).unwrap();
        let mut request = autosave_request(
            source,
            target.clone(),
            opened,
            dir.join("opened"),
            Edits::default(),
        );
        request.expected = None;
        let mut worker = Worker { engine: None, decoded: Lru::new(0), com: false, long: None, readers: Arc::default() };
        worker.autosave(request).result.unwrap();
        let png = std::fs::read(&target).unwrap();
        assert!(png.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]), "the destination is encoded as PNG");
        assert_ne!(png, jpeg, "a different destination format must not copy the JPEG stream");
        assert_eq!(crate::imaging::decode(&target, 100, 100).unwrap().source_width, 2);
        drop(worker);
        std::fs::remove_dir_all(dir).unwrap();
        unsafe { CoUninitialize() };
    }

    #[test]
    fn overwrite_autosave_rerender_and_save_copy_apply_image_edits_once() {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap() };
        let dir = temp("single-apply-image");
        let source = dir.join("photo.png");
        let output = dir.join("photo-copy.png");
        let frame = Frame {
            width: 2,
            height: 1,
            pixels: vec![0, 0, 255, 255, 0, 255, 0, 255],
            page_count: 1,
            source_width: 2,
            source_height: 1,
        };
        crate::imaging::export_frame(&frame, &source).unwrap();
        let opened = disk::Stamp::of(&source).unwrap();
        let edits = Edits {
            image: vec![ImageEdit::RotateRight],
            pdf: Vec::new(),
            dirty: true,
        };
        let mut worker = Worker { engine: None, decoded: Lru::new(0), com: true, long: None, readers: Arc::default() };
        let saved = worker.autosave(autosave_request(
            source.clone(),
            source.clone(),
            opened,
            dir.join("opened"),
            edits.clone(),
        ));
        saved.result.unwrap();
        let snapshot = saved.snapshot.unwrap();
        assert_eq!(crate::imaging::decode(&source, 100, 100).unwrap().source_width, 1, "overwrite contains one rotation");

        let mut sessions = HashMap::new();
        sessions.insert(source.clone(), edits);
        let mut sources = HashMap::new();
        sources.insert(source.clone(), snapshot);
        let request = Request {
            generation: 1,
            path: source.clone(),
            page: 0,
            delta: 0,
            width: 100,
            height: 100,
            sessions,
            sources,
        };
        let (sender, receiver) = mpsc::channel();
        let events = Events { sender, window: 0 };
        assert!(worker.run(Job::Render(request.clone()), Vec::new(), &events));
        let Event::Render(rendered) = receiver.recv().unwrap() else { panic!("expected render") };
        let rendered = rendered.result.unwrap();
        assert_eq!((rendered.source_width, rendered.source_height), (1, 2), "rerender applies the rotation once");

        assert!(worker.run(Job::Save(request, output.clone(), SaveKind::Copy), Vec::new(), &events));
        let Event::Saved(_, _, _, saved) = receiver.recv().unwrap() else { panic!("expected save") };
        saved.unwrap();
        let copied = crate::imaging::decode(&output, 100, 100).unwrap();
        assert_eq!((copied.source_width, copied.source_height), (1, 2), "Save a copy applies the rotation once");
        drop(worker);
        std::fs::remove_dir_all(dir).unwrap();
        unsafe { CoUninitialize() };
    }

    #[test]
    fn overwrite_autosave_batch_uses_snapshots_and_current_recipe_for_all_files() {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap() };
        let dir = temp("single-apply-batch");
        let source = dir.join("photo.png");
        let second = dir.join("second.png");
        let second_snapshot = dir.join("second-opened.png");
        let output = dir.join("converted");
        std::fs::create_dir_all(&output).unwrap();
        let frame = Frame {
            width: 2,
            height: 1,
            pixels: vec![0, 0, 255, 255, 0, 255, 0, 255],
            page_count: 1,
            source_width: 2,
            source_height: 1,
        };
        crate::imaging::export_frame(&frame, &source).unwrap();
        let second_frame = Frame {
            width: 3,
            source_width: 3,
            pixels: [255, 0, 0, 255].repeat(3),
            ..frame.clone()
        };
        crate::imaging::export_frame(&second_frame, &second).unwrap();
        std::fs::copy(&second, &second_snapshot).unwrap();
        let replaced_second = Frame {
            width: 4,
            source_width: 4,
            pixels: [0, 255, 0, 255].repeat(4),
            ..frame.clone()
        };
        std::fs::remove_file(&second).unwrap();
        crate::imaging::export_frame(&replaced_second, &second).unwrap();
        let edits = Edits { image: vec![ImageEdit::RotateRight], pdf: Vec::new(), dirty: true };
        let mut worker = Worker { engine: None, decoded: Lru::new(0), com: true, long: None, readers: Arc::default() };
        let saved = worker.autosave(autosave_request(
            source.clone(),
            source.clone(),
            disk::Stamp::of(&source).unwrap(),
            dir.join("opened"),
            edits.clone(),
        ));
        saved.result.unwrap();
        let mut sessions = HashMap::new();
        sessions.insert(source.clone(), edits);
        sessions.insert(
            second.clone(),
            Edits {
                image: vec![ImageEdit::RotateRight, ImageEdit::RotateRight],
                ..Edits::default()
            },
        );
        let mut sources = HashMap::new();
        sources.insert(source.clone(), saved.snapshot.unwrap());
        sources.insert(second.clone(), second_snapshot);
        let request = Request {
            generation: 1,
            path: source.clone(),
            page: 0,
            delta: 0,
            width: 1,
            height: 1,
            sessions,
            sources,
        };
        assert!(
            !runs_on_task_worker(&Job::Batch(
                request.clone(),
                output.clone(),
                "png".into(),
                Arc::new(AtomicBool::new(false)),
                Some(vec![source.clone(), second.clone()]),
            )),
            "snapshot-backed batch waits behind snapshot creation on the document worker"
        );
        let (sender, receiver) = mpsc::channel();
        let events = Events { sender, window: 0 };
        assert!(worker.run(
            Job::Batch(
                request,
                output.clone(),
                "png".into(),
                Arc::new(AtomicBool::new(false)),
                Some(vec![source.clone(), second.clone()]),
            ),
            Vec::new(),
            &events,
        ));
        assert!(worker.step(&events));
        assert!(matches!(receiver.recv().unwrap(), Event::Progress(_)));
        assert!(worker.step(&events));
        assert!(matches!(receiver.recv().unwrap(), Event::Progress(_)));
        assert!(worker.step(&events));
        let Event::Finished(result) = receiver.recv().unwrap() else { panic!("expected finished") };
        result.unwrap();
        let converted = output.join("photo.png");
        let frame = crate::imaging::decode(&converted, 100, 100).unwrap();
        assert_eq!((frame.source_width, frame.source_height), (1, 2), "batch applies the rotation once");
        let converted_second = crate::imaging::decode(&output.join("second.png"), 100, 100).unwrap();
        assert_eq!(
            (converted_second.source_width, converted_second.source_height),
            (1, 3),
            "batch applies the current image recipe to every selected snapshot"
        );
        drop(worker);
        std::fs::remove_dir_all(dir).unwrap();
        unsafe { CoUninitialize() };
    }

    #[test]
    fn pdf_autosave_replays_edits_from_the_opened_snapshot() {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/20-pages.pdf");
        assert!(fixture.is_file(), "run tools/make-fixtures.ps1 before this test");
        let dir = temp("pdf");
        let source = dir.join("document.pdf");
        std::fs::copy(&fixture, &source).unwrap();
        let opened_bytes = std::fs::read(&source).unwrap();
        let opened = disk::Stamp::of(&source).unwrap();
        let edits = Edits {
            image: Vec::new(),
            pdf: vec![PdfEdit::RotateRight { page: 0 }],
            dirty: true,
        };
        let mut worker = Worker { engine: None, decoded: Lru::new(0), com: false, long: None, readers: Arc::default() };
        let saved = worker.autosave(autosave_request(
            source.clone(),
            source.clone(),
            opened,
            dir.join("opened"),
            edits,
        ));
        saved.result.unwrap();
        let snapshot = saved.snapshot.unwrap();
        assert_eq!(std::fs::read(&snapshot).unwrap(), opened_bytes, "snapshot keeps exactly the opened bytes");
        let page = worker.pdf().unwrap().render(&source, 0, 1200, 1200).unwrap();
        assert!(page.width > page.height, "the saved first page is rotated");

        let output = dir.join("copy.pdf");
        let mut sessions = HashMap::new();
        sessions.insert(
            source.clone(),
            Edits { pdf: vec![PdfEdit::RotateRight { page: 0 }], ..Edits::default() },
        );
        let mut sources = HashMap::new();
        sources.insert(source.clone(), snapshot);
        let request = Request {
            generation: 2,
            path: source.clone(),
            page: 0,
            delta: 0,
            width: 1,
            height: 1,
            sessions,
            sources,
        };
        let (sender, receiver) = mpsc::channel();
        let events = Events { sender, window: 0 };
        assert!(worker.run(Job::Open(request.clone()), Vec::new(), &events));
        let Event::Pages(opened) = receiver.recv().unwrap() else { panic!("expected pages") };
        let sizes = opened.result.unwrap();
        assert!(sizes[0][0] > sizes[0][1], "rerender applies the PDF rotation once");
        assert!(worker.run(Job::Save(request, output.clone(), SaveKind::Copy), Vec::new(), &events));
        let Event::Saved(_, _, _, saved) = receiver.recv().unwrap() else { panic!("expected save") };
        saved.unwrap();
        let copied = worker.pdf().unwrap().render(&output, 0, 1200, 1200).unwrap();
        assert!(copied.width > copied.height, "Save a copy applies the PDF rotation once");
        drop(worker);
        std::fs::remove_dir_all(dir).unwrap();
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
            sources: HashMap::new(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        assert!(runs_on_task_worker(&Job::Layer(request("a.png"))), "OCR layer");
        assert!(runs_on_task_worker(&Job::Text(request("a.png"), None)), "OCR copy");
        assert!(!runs_on_task_worker(&Job::Layer(request("a.pdf"))), "PDF text uses PDFium");
        assert!(!runs_on_task_worker(&Job::Text(request("a.pdf"), None)), "PDF copy uses PDFium");
        assert!(runs_on_task_worker(&Job::Background(request("a.png"), PathBuf::from("b.png"))));
        assert!(runs_on_task_worker(&Job::Batch(request("a.png"), PathBuf::from("out"), "webp".into(), cancel.clone(), None)));
        assert!(!runs_on_task_worker(&Job::Batch(request("a.png"), PathBuf::from("out"), "pdf".into(), cancel, None)));
        assert!(runs_on_task_worker(&Job::Save(request("a.png"), PathBuf::from("b.jpg"), SaveKind::Copy)));
        assert!(!runs_on_task_worker(&Job::Save(request("a.png"), PathBuf::from("b.pdf"), SaveKind::Copy)));
        assert!(!runs_on_task_worker(&Job::Save(request("a.pdf"), PathBuf::from("b.pdf"), SaveKind::ExtractPage)));
        let stamp = disk::Stamp { len: 1, modified: None, created: None };
        assert!(!runs_on_task_worker(&Job::Autosave(AutosaveRequest {
            path: PathBuf::from("a.png"),
            session_id: 1,
            revision: 1,
            opened: stamp,
            snapshot: None,
            snapshot_dir: PathBuf::from("opened"),
            target: PathBuf::from("a.png"),
            expected: Some(stamp),
            force: false,
            edits: Edits::default(),
        })));
        assert!(!runs_on_task_worker(&Job::Render(request("a.png"))));
        assert!(!runs_on_task_worker(&Job::Open(request("a.pdf"))));
        assert!(!runs_on_task_worker(&Job::Find(request("a.pdf"), "x".into())));
        assert!(!runs_on_task_worker(&Job::Fields(request("a.pdf"))));
        let mut snapshot_backed = request("a.png");
        snapshot_backed.sources.insert(PathBuf::from("a.png"), PathBuf::from("opened/a.png"));
        assert!(runs_on_task_worker(&Job::Layer(snapshot_backed.clone())), "snapshot-backed OCR remains off the document worker");
        assert!(runs_on_task_worker(&Job::Text(snapshot_backed.clone(), None)), "snapshot-backed OCR copy remains off the document worker");
        assert!(!runs_on_task_worker(&Job::Background(snapshot_backed.clone(), PathBuf::from("b.png"))));
        assert!(!runs_on_task_worker(&Job::Save(
            snapshot_backed,
            PathBuf::from("b.png"),
            SaveKind::Copy,
        )));
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
            sources: HashMap::new(),
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

    #[test]
    fn image_cache_revalidates_replaced_and_deleted_sources() {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap(); }
        let folder = std::env::temp_dir().join(format!("pfw-cache-revision-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&folder).unwrap();
        let first = folder.join("001.png");
        let neighbor = folder.join("002.png");
        let replacement = folder.join("replacement.png");
        let frame = |width, pixel: [u8; 4]| Frame { width, height: 1, pixels: pixel.repeat(width as usize), page_count: 1, source_width: width, source_height: 1 };
        crate::imaging::export_frame(&frame(1, [0, 0, 255, 255]), &first).unwrap();
        crate::imaging::export_frame(&frame(1, [0, 0, 255, 255]), &neighbor).unwrap();
        let mut worker = Worker { engine: None, decoded: Lru::new(0), com: true, long: None, readers: Arc::default() };
        let item = Item { key: Key { doc: doc_id(&first), work: Work::Predecode { delta: 1, width: 100, height: 100 } }, path: first.clone(), edits: Arc::default(), scale: 0.0, region: [0; 4] };
        worker.queued(&item);
        let (old, cached) = worker.decode(&neighbor, 100, 100, &[]);
        assert!(cached);
        assert_eq!(old.unwrap().source_width, 1);
        crate::imaging::export_frame(&frame(2, [255, 0, 0, 255]), &replacement).unwrap();
        std::fs::remove_file(&neighbor).unwrap();
        std::fs::rename(&replacement, &neighbor).unwrap();
        let (new, cached) = worker.decode(&neighbor, 100, 100, &[]);
        assert!(!cached, "external replacement must invalidate navigation's cache");
        assert_eq!(new.unwrap().source_width, 2);
        assert!(worker.decode(&neighbor, 100, 100, &[]).1, "unchanged files stay cached");
        crate::imaging::export_frame(&frame(3, [0, 255, 0, 255]), &replacement).unwrap();
        std::fs::remove_file(&neighbor).unwrap();
        std::fs::rename(&replacement, &neighbor).unwrap();
        worker.queued(&item);
        let (new, cached) = worker.decode(&neighbor, 100, 100, &[]);
        assert!(cached, "predecode refreshes an obsolete cache entry");
        assert_eq!(new.unwrap().source_width, 3);
        std::fs::remove_file(&neighbor).unwrap();
        let (missing, cached) = worker.decode(&neighbor, 100, 100, &[]);
        assert!(!cached && missing.is_err(), "deleted images must not return cached pixels");
        drop(worker);
        std::fs::remove_file(first).unwrap();
        std::fs::remove_dir(folder).unwrap();
        unsafe { CoUninitialize(); }
    }

    /// Next image after its neighbors were pre-decoded, through the real
    /// document worker and WIC. The window adds one upload and one draw.
    #[test]
    fn next_image_comes_from_the_predecode_cache_headless() {
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/image-24mp.jpg");
        if !source.is_file() {
            eprintln!("skipped: fixtures are missing; run tools/make-fixtures.ps1");
            return;
        }
        let folder = std::env::temp_dir().join(format!("pfw-next-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let folder = std::fs::canonicalize(folder).unwrap();
        for n in 1..=3 {
            std::fs::copy(&source, folder.join(format!("{n:03}.jpg"))).unwrap();
        }
        let first = folder.join("001.jpg");
        let mut workers = Workers::start(HWND::default()).unwrap();
        let (width, height) = (1100, 700);
        let request = |delta: i32| Request {
            generation: 1,
            path: first.clone(),
            page: 0,
            delta,
            width,
            height,
            sessions: HashMap::new(),
            sources: HashMap::new(),
        };
        let timeout = std::time::Duration::from_secs(30);
        let started = std::time::Instant::now();
        assert!(workers.send(Job::Render(request(0))));
        let Ok(Event::Render(shown)) = workers.receiver.recv_timeout(timeout) else { panic!("no image") };
        let cold = started.elapsed().as_secs_f64() * 1000.0;
        let frame = shown.result.unwrap();
        assert_eq!((frame.source_width, frame.source_height, shown.from_predecode), (6000, 4000, false));
        let neighbor = |delta: i32| Item {
            key: Key { doc: doc_id(&first), work: Work::Predecode { delta, width, height } },
            path: first.clone(),
            edits: Arc::default(),
            scale: 0.0,
            region: [0; 4],
        };
        workers.request(vec![neighbor(1), neighbor(-1)]);
        for _ in 0..2 {
            let Ok(Event::Done(item, Outcome::Predecoded)) = workers.receiver.recv_timeout(timeout) else { panic!("no pre-decode") };
            workers.delivered(&item.key);
        }
        let started = std::time::Instant::now();
        assert!(workers.send(Job::Render(request(1))));
        let Ok(Event::Render(next)) = workers.receiver.recv_timeout(timeout) else { panic!("no next image") };
        let warm = started.elapsed().as_secs_f64() * 1000.0;
        assert!(next.from_predecode && next.navigation);
        assert_eq!(next.path, folder.join("002.jpg"));
        assert!(next.result.is_ok());
        println!("24 MP JPEG at 1100 x 700: first decode {cold:.1} ms; next image from pre-decode {warm:.2} ms");
        assert!(warm < 50.0, "PRD: next image under 50 ms");
        workers.stop();
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn navigation_reuses_a_predecode_before_ui_acknowledgement() {
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/image-24mp.jpg");
        assert!(source.is_file(), "run tools/make-fixtures.ps1 before this headless test");
        let folder = std::env::temp_dir().join(format!("pfw-next-inflight-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let folder = std::fs::canonicalize(folder).unwrap();
        for name in ["001.jpg", "002.jpg"] {
            std::fs::copy(&source, folder.join(name)).unwrap();
        }
        let first = folder.join("001.jpg");
        let key = Key { doc: doc_id(&first), work: Work::Predecode { delta: 1, width: 1100, height: 700 } };
        let mut workers = Workers::start(HWND::default()).unwrap();
        workers.request(vec![Item { key, path: first.clone(), edits: Arc::default(), scale: 0.0, region: [0; 4] }]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        // Wait for dispatch, not completion. The UI has not consumed or
        // acknowledged the pre-decode when the user requests navigation.
        while !workers.queue.lock().unwrap().taken.contains(&key) {
            assert!(std::time::Instant::now() < deadline, "pre-decode was not dispatched");
            std::thread::yield_now();
        }
        assert!(workers.send(Job::Render(Request {
            generation: 2,
            path: first,
            page: 0,
            delta: 1,
            width: 1100,
            height: 700,
            sessions: HashMap::new(),
            sources: HashMap::new(),
        })));
        let next = loop {
            match workers.receiver.recv_timeout(std::time::Duration::from_secs(30)).unwrap() {
                Event::Done(item, Outcome::Predecoded) => workers.delivered(&item.key),
                Event::Render(next) => break next,
                _ => panic!("unexpected worker event"),
            }
        };
        assert!(next.from_predecode && next.navigation, "navigation must reuse the unacknowledged predecode");
        assert_eq!(next.path, folder.join("002.jpg"));
        assert!(next.result.is_ok());
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
        let request = Request {
            generation: 7,
            path: path.clone(),
            page: 0,
            delta: 0,
            width: 1,
            height: 1,
            sessions: HashMap::new(),
            sources: HashMap::new(),
        };
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
