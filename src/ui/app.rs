//! Window-thread state, tabs, render scheduling, and worker results.
use super::{
    bench::Bench,
    cache::{Lru, TILE_BUDGET},
    commands::{self, Command, Ctx},
    document::{self, PdfView, Pinch, Stats, Tiles},
    render::{fonts, measure, Renderer},
    sheet::{self, Sheet},
    theme::Theme,
    view::{ViewMode, Zoom},
    widgets::{self, Layout, Scope, SheetView, SidebarList, WidgetId},
    worker::{is_pdf, AutosaveRequest, Autosaved, Edits, Event, Job, Key, Outcome, Request, Work, Workers},
};
use crate::model::{AnnotationKind, Frame, SearchHit, TextLayer};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
    time::{Duration, Instant},
};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::HWND,
        Graphics::Gdi::InvalidateRect,
        UI::WindowsAndMessaging::SetWindowTextW,
    },
};

pub(super) const EMPTY_STATUS: &str = "Open a PDF or image with Ctrl+O.";
const TOOLTIP_DELAY: Duration = Duration::from_millis(500);
/// Fluent "fast" duration for entering and exiting elements.
/// https://learn.microsoft.com/windows/apps/design/motion/timing-and-easing
const SLIDE: Duration = Duration::from_millis(167);
const AUTOSAVE_DELAY: Duration = Duration::from_millis(500);

/// Linear slide progress from `from` toward open (1) or closed (0).
pub(super) fn slide_progress(from: f32, open: bool, elapsed: Duration, animate: bool) -> f32 {
    let to = if open { 1.0 } else { 0.0 };
    if !animate {
        return to;
    }
    let t = (elapsed.as_secs_f32() / SLIDE.as_secs_f32()).min(1.0);
    from + (to - from) * t
}

/// Cubic ease-out: decelerates when opening and accelerates when closing,
/// close to Fluent's cubic-bezier(0, 0, 0, 1) entrance curve.
pub(super) fn ease(progress: f32) -> f32 {
    1.0 - (1.0 - progress.clamp(0.0, 1.0)).powi(3)
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum SaveStatus {
    Clean,
    Edited,
    Saving,
    Saved,
    Conflict,
    Failed(String),
}

#[derive(Clone, Debug)]
pub(super) struct SaveSession {
    pub(super) id: u64,
    pub(super) opened: super::disk::Stamp,
    pub(super) snapshot: Option<PathBuf>,
    pub(super) target: Option<PathBuf>,
    pub(super) expected: Option<super::disk::Stamp>,
    pub(super) revision: u64,
    pub(super) saved_revision: u64,
    pub(super) due: Option<Instant>,
    pub(super) in_flight: Option<u64>,
    pub(super) force: bool,
    pub(super) status: SaveStatus,
}

impl SaveSession {
    fn new(id: u64, opened: super::disk::Stamp) -> Self {
        Self {
            id,
            opened,
            snapshot: None,
            target: None,
            expected: None,
            revision: 0,
            saved_revision: 0,
            due: None,
            in_flight: None,
            force: false,
            status: SaveStatus::Clean,
        }
    }

    /// Records a recipe change. A conflict remains paused until the user
    /// chooses whether to overwrite or save elsewhere.
    pub(super) fn edited(&mut self, now: Instant) {
        self.revision = self.revision.wrapping_add(1);
        self.due = Some(now + AUTOSAVE_DELAY);
        if !matches!(self.status, SaveStatus::Conflict) {
            self.status = SaveStatus::Edited;
        }
    }

    pub(super) fn configure(&mut self, target: PathBuf, expected: Option<super::disk::Stamp>) {
        self.target = Some(target);
        self.expected = expected;
        self.force = false;
    }

    pub(super) fn resolve_conflict(&mut self, target: PathBuf, expected: Option<super::disk::Stamp>, force: bool) {
        self.target = Some(target);
        self.expected = expected;
        self.force = force;
        self.status = SaveStatus::Edited;
        self.due = Some(Instant::now());
    }

    pub(super) fn begin(
        &mut self,
        path: PathBuf,
        edits: Edits,
        snapshot_dir: PathBuf,
        force: bool,
    ) -> Option<AutosaveRequest> {
        if self.in_flight.is_some() || matches!(self.status, SaveStatus::Conflict) {
            return None;
        }
        let target = self.target.clone()?;
        let revision = self.revision;
        let existing_snapshot = self.snapshot.clone();
        if self.snapshot.is_none() {
            self.snapshot = path.file_name().map(|name| snapshot_dir.join(name));
        }
        self.in_flight = Some(revision);
        self.due = None;
        self.status = SaveStatus::Saving;
        Some(AutosaveRequest {
            path,
            session_id: self.id,
            revision,
            opened: self.opened,
            snapshot: existing_snapshot,
            snapshot_dir,
            target,
            expected: self.expected,
            force,
            edits,
        })
    }

    /// Applies only the completion currently in flight. An older successful
    /// revision advances the destination stamp but leaves newer edits dirty.
    fn apply(&mut self, event: &Autosaved) -> Option<bool> {
        if event.session_id != self.id
            || self.in_flight != Some(event.revision)
            || self.target.as_ref() != Some(&event.target)
        {
            return None;
        }
        self.in_flight = None;
        self.snapshot = event.snapshot.clone();
        match &event.result {
            Ok(stamp) => {
                self.expected = Some(*stamp);
                self.force = false;
                self.saved_revision = event.revision;
                let caught_up = self.revision == event.revision;
                self.status = if caught_up { SaveStatus::Saved } else { SaveStatus::Edited };
                if !caught_up {
                    self.due = Some(Instant::now());
                }
                Some(caught_up)
            }
            Err(super::disk::Failure::Changed) => {
                self.status = SaveStatus::Conflict;
                Some(false)
            }
            Err(super::disk::Failure::Error(error)) => {
                self.status = SaveStatus::Failed(error.clone());
                Some(false)
            }
        }
    }
}

pub(super) struct State {
    pub(super) workers: Workers,
    pub(super) path: Option<PathBuf>,
    pub(super) page: u32,
    pub(super) generation: u64,
    pub(super) frame: Option<Frame>,
    pub(super) status: String,
    pub(super) pending: bool,
    pub(super) painted: bool,
    pub(super) due: Option<Instant>,
    pub(super) marked: bool,
    pub(super) sessions: HashMap<PathBuf, Edits>,
    pub(super) saves: HashMap<PathBuf, SaveSession>,
    next_save_session: u64,
    save_dispatch_paused: bool,
    /// Zoom for the open file. An image keeps its frame and draws it at the
    /// new size until a frame decoded for that size arrives.
    pub(super) zoom: Zoom,
    pub(super) view_mode: ViewMode,
    /// Image offset from the centered position.
    pub(super) pan: (f32, f32),
    /// The open PDF; None while an image shows.
    pub(super) pdf: Option<PdfView>,
    /// Page tiles and thumbnails as bitmaps of the current renderer.
    pub(super) cache: Tiles,
    /// Rendered tiles waiting for upload in the next paint.
    pub(super) arrived: Vec<(Key, Frame, bool)>,
    /// Keys of the work list last sent to the document worker.
    pub(super) sent: Vec<Key>,
    pub(super) stats: Stats,
    /// The view showed complete content at least once.
    pub(super) content_drawn: bool,
    /// The last image came from the neighbor pre-decode cache.
    pub(super) from_predecode: bool,
    pub(super) zoom_select: bool,
    pub(super) bench: Option<Bench>,
    pub(super) drag: Option<(f32, f32)>,
    pub(super) crop: bool,
    pub(super) selection: Option<(f32, f32, f32, f32)>,
    pub(super) text_layers: HashMap<u32, TextLayer>,
    pub(super) text_requested: HashSet<u32>,
    pub(super) text_failed: HashMap<u32, String>,
    pub(super) text_selection: Option<super::text::Selection>,
    pub(super) text_drag: bool,
    pub(super) text_click: Option<(u64, (f32, f32), u8)>,
    pub(super) find_query: String,
    pub(super) find_hits: Vec<SearchHit>,
    pub(super) find_index: usize,
    pub(super) image_rect: widgets::Rect,
    pub(super) exporting: bool,
    pub(super) displayed: Option<(PathBuf, u32)>,
    pub(super) render_failed: bool,
    pub(super) markup: Option<AnnotationKind>,
    pub(super) ink: Vec<[f32; 2]>,
    pub(super) markup_text: String,
    pub(super) signature: Option<Vec<[f32; 2]>>,
    pub(super) cancel: Option<Arc<AtomicBool>>,
    pub(super) slideshow: Option<Instant>,
    pub(super) tabs: Vec<PathBuf>,
    pub(super) password_attempts: HashMap<PathBuf, u32>,
    pub(super) views: HashMap<PathBuf, (u32, Zoom, (f32, f32))>,
    // Window and chrome.
    pub(super) size: (f32, f32),
    pub(super) scale: f32,
    pub(super) text_scale: f32,
    pub(super) theme: Theme,
    pub(super) renderer: Option<Renderer>,
    pub(super) maximized: bool,
    pub(super) active: bool,
    pub(super) sidebar_open: bool,
    pub(super) sidebar_tab: usize,
    /// Scroll offset and last used row of each sidebar tab.
    pub(super) sidebar_scroll: [f32; 3],
    pub(super) sidebar_rows: [usize; 3],
    pub(super) organize: super::organize::Organize,
    pub(super) markup_open: bool,
    /// Markup bar slide: when it started and the progress it started from.
    pub(super) markup_since: Option<Instant>,
    pub(super) markup_from: f32,
    /// Windows "Animation effects" setting (SPI_GETCLIENTAREAANIMATION).
    pub(super) animations: bool,
    pub(super) hover: Option<WidgetId>,
    pub(super) hover_since: Option<Instant>,
    pub(super) tooltip: Option<WidgetId>,
    pub(super) pressed: Option<WidgetId>,
    pub(super) focus: Option<WidgetId>,
    /// Focus rectangles show after keyboard use only, as in Windows.
    pub(super) focus_visible: bool,
    pub(super) keytips: Option<Scope>,
    /// Alt went down with no other key yet; releasing it shows keytips.
    pub(super) alt_armed: bool,
    pub(super) caption_hover: Option<WidgetId>,
    pub(super) caption_pressed: Option<WidgetId>,
    pub(super) touches: Vec<(u32, (f32, f32))>,
    pub(super) pinch: Option<Pinch>,
    pub(super) sheet: Option<Sheet>,
    /// Accessibility and UISettings start after first content (see tick).
    pub(super) started: bool,
    pub(super) tools: super::imagetools::Tools,
    pub(super) find: super::findbar::FindBar,
    pub(super) messages: super::infobar::Messages,
    /// Recent files for the empty window, newest first.
    pub(super) recent: Vec<PathBuf>,
}

thread_local! { static STATE: RefCell<Option<State>> = const { RefCell::new(None) }; }

pub(super) fn install(state: State) {
    STATE.with(|cell| *cell.borrow_mut() = Some(state));
}
pub(super) fn uninstall() {
    let state = STATE.with(|cell| cell.borrow_mut().take());
    drop(state);
}

/// Runs `f` on the window state. Returns None during reentrant calls (a
/// message sent while the state is borrowed), so handlers never panic.
pub(super) fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|cell| cell.try_borrow_mut().ok().and_then(|mut s| s.as_mut().map(f)))
}

pub(super) fn file_name(path: &std::path::Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

impl State {
    /// The first path opens at once; every path gets a tab.
    pub(super) fn new(workers: Workers, paths: &[PathBuf], size: (f32, f32), scale: f32, text_scale: f32, theme: Theme) -> Self {
        let path = paths.first().cloned();
        let mut state = State {
            workers,
            focus: path.as_ref().map(|_| WidgetId::Document),
            status: if path.is_none() { EMPTY_STATUS.into() } else { "Opening...".into() },
            path,
            page: 0,
            generation: 0,
            frame: None,
            pending: false,
            painted: false,
            due: Some(Instant::now()),
            marked: false,
            sessions: HashMap::new(),
            saves: HashMap::new(),
            next_save_session: 1,
            save_dispatch_paused: false,
            zoom: Zoom::Fit,
            view_mode: ViewMode::Continuous,
            pan: (0.0, 0.0),
            pdf: None,
            cache: Lru::new(TILE_BUDGET),
            arrived: Vec::new(),
            sent: Vec::new(),
            stats: Stats::default(),
            content_drawn: false,
            from_predecode: false,
            zoom_select: false,
            bench: None,
            drag: None,
            crop: false,
            selection: None,
            text_layers: HashMap::new(),
            text_requested: HashSet::new(),
            text_failed: HashMap::new(),
            text_selection: None,
            text_drag: false,
            text_click: None,
            find_query: String::new(),
            find_hits: Vec::new(),
            find_index: 0,
            image_rect: widgets::Rect::default(),
            exporting: false,
            displayed: None,
            render_failed: false,
            markup: None,
            ink: Vec::new(),
            markup_text: String::new(),
            signature: None,
            cancel: None,
            slideshow: None,
            tabs: Vec::new(),
            password_attempts: HashMap::new(),
            views: HashMap::new(),
            size,
            scale,
            text_scale,
            theme,
            renderer: None,
            maximized: false,
            active: true,
            sidebar_open: false,
            sidebar_tab: 0,
            sidebar_scroll: [0.0; 3],
            sidebar_rows: [0; 3],
            organize: Default::default(),
            markup_open: false,
            markup_since: None,
            markup_from: 0.0,
            animations: super::theme::animations_enabled(),
            hover: None,
            hover_since: None,
            tooltip: None,
            pressed: None,
            focus_visible: false,
            keytips: None,
            alt_armed: false,
            caption_hover: None,
            caption_pressed: None,
            touches: Vec::new(),
            pinch: None,
            sheet: None,
            started: false,
            tools: Default::default(),
            find: Default::default(),
            messages: Default::default(),
            recent: if paths.is_empty() { super::empty::load() } else { Vec::new() },
        };
        add_tabs(&mut state, paths);
        state
    }
    fn opened_for_save(&mut self, path: PathBuf, stamp: super::disk::Stamp) {
        if self.saves.contains_key(&path) {
            return;
        }
        let id = self.next_save_session;
        self.next_save_session = self.next_save_session.wrapping_add(1).max(1);
        self.saves.insert(path, SaveSession::new(id, stamp));
    }
    #[allow(dead_code)] // The first-edit command path calls this in the next stage.
    pub(super) fn edited_for_save(&mut self, path: &PathBuf, now: Instant) {
        if let Some(save) = self.saves.get_mut(path) {
            save.edited(now);
        }
    }
    fn autosaved(&mut self, event: Autosaved) -> bool {
        let active = self.path.as_ref() == Some(&event.path);
        let Some(save) = self.saves.get_mut(&event.path) else {
            return false;
        };
        let Some(caught_up) = save.apply(&event) else {
            return false;
        };
        if caught_up {
            if let Some(edits) = self.sessions.get_mut(&event.path) {
                edits.dirty = false;
            }
        }
        if active {
            self.status = match &save.status {
                SaveStatus::Saved => "Saved".into(),
                SaveStatus::Edited => "Edited".into(),
                SaveStatus::Conflict => "Autosave paused because the file changed outside Preview.".into(),
                SaveStatus::Failed(error) => format!("Save failed: {error}"),
                SaveStatus::Saving => "Saving...".into(),
                SaveStatus::Clean => self.status.clone(),
            };
        }
        super::infobar::save_problem(self, &event.path);
        true
    }
    pub(super) fn is_pdf(&self) -> bool {
        self.path.as_ref().is_some_and(|p| is_pdf(p))
    }
    pub(super) fn sidebar_visible(&self) -> bool {
        self.sidebar_open && self.is_pdf()
    }
    pub(super) fn active_tab(&self) -> Option<usize> {
        self.path.as_ref().and_then(|p| self.tabs.iter().position(|t| t == p))
    }
    pub(super) fn tool(&self) -> Option<Command> {
        if self.signature.is_some() {
            return Some(Command::PlaceSignature);
        }
        let kind = self.markup?;
        commands::MARKUP_TOOLS.iter().copied().find(|c| commands::annotation(*c) == Some(kind))
    }
    fn markup_progress(&self) -> f32 {
        match self.markup_since {
            Some(since) => slide_progress(self.markup_from, self.markup_open, since.elapsed(), self.animations),
            None if self.markup_open => 1.0,
            None => 0.0,
        }
    }
    /// Opens or closes the markup bar with a slide.
    pub(super) fn set_markup(&mut self, open: bool) {
        if open != self.markup_open {
            self.markup_from = self.markup_progress();
            self.markup_open = open;
            self.markup_since = Some(Instant::now());
        }
    }
    pub(super) fn page_count(&self) -> u32 {
        match (&self.pdf, &self.frame) {
            (Some(v), _) => v.sizes.len() as u32,
            (None, Some(frame)) => frame.page_count,
            _ => 0,
        }
    }
    pub(super) fn ctx(&self) -> Ctx {
        let pdf = self.is_pdf();
        Ctx {
            has_frame: self.frame.is_some() || self.pdf.is_some(),
            pending: self.pending,
            failed: self.render_failed,
            pdf,
            saving: self.exporting,
            can_previous: !pdf || self.page > 0,
            can_next: !pdf || self.page + 1 < self.page_count(),
            tabs: self.tabs.len(),
            sidebar_open: self.sidebar_visible(),
            markup_open: self.markup_open,
            crop: self.crop,
            tool: self.tool(),
            zoom: self.zoom,
            view: self.view_mode,
            zoom_select: self.zoom_select,
        }
    }
    /// "Page 3 of 20" or "1920 × 1080 pixels".
    pub(super) fn subtitle(&self) -> String {
        match (&self.pdf, &self.frame) {
            (Some(v), _) => format!("Page {} of {}", self.page + 1, v.sizes.len()),
            (None, Some(frame)) if self.is_pdf() => format!("Page {} of {}", self.page + 1, frame.page_count),
            (None, Some(frame)) => format!("{} × {} pixels", frame.source_width, frame.source_height),
            _ => String::new(),
        }
    }
    pub(super) fn visible_status(&self) -> String {
        let Some(save) = self.path.as_ref().and_then(|path| self.saves.get(path)) else {
            return self.status.clone();
        };
        match &save.status {
            SaveStatus::Clean => self.status.clone(),
            SaveStatus::Edited => "Edited".into(),
            SaveStatus::Saving => "Saving...".into(),
            SaveStatus::Saved => "Saved".into(),
            SaveStatus::Conflict => "Autosave paused because the file changed outside Preview.".into(),
            SaveStatus::Failed(error) => format!("Save failed: {error}"),
        }
    }
    pub(super) fn tab_close_state(&self, path: &PathBuf) -> (bool, bool) {
        let save = self.saves.get(path);
        (
            self.sessions.get(path).is_some_and(|edits| edits.dirty)
                || save.is_some_and(|save| save.revision != save.saved_revision),
            save.is_some_and(|save| save.in_flight.is_some()) || self.workers.reads_snapshot(path),
        )
    }
    pub(super) fn window_close_state(&self) -> (bool, bool) {
        (
            self.sessions.values().any(|edits| edits.dirty)
                || self.saves.values().any(|save| save.revision != save.saved_revision),
            self.exporting
                || self.saves.values().any(|save| save.in_flight.is_some())
                || self.workers.reads_any_snapshot(),
        )
    }
    pub(super) fn pause_save_dispatch(&mut self, paused: bool) {
        self.save_dispatch_paused = paused;
    }
    pub(super) fn cleanup_snapshots(&mut self) {
        for snapshot in self.saves.values().filter_map(|save| save.snapshot.as_ref()) {
            if let Some(folder) = snapshot.parent() {
                let _ = std::fs::remove_dir_all(folder);
            }
        }
    }

    fn autosave_requests(&mut self, now: Instant) -> Vec<AutosaveRequest> {
        if self.save_dispatch_paused {
            return Vec::new();
        }
        let due: Vec<PathBuf> = self
            .saves
            .iter()
            .filter(|(_, save)| save.due.is_some_and(|due| due <= now) && save.in_flight.is_none())
            .map(|(path, _)| path.clone())
            .collect();
        let base = super::disk::app_dir().unwrap_or_else(|| std::env::temp_dir().join("PreviewForWindows"));
        let mut requests = Vec::new();
        for path in due {
            let edits = self.sessions.get(&path).cloned().unwrap_or_default();
            let Some(save) = self.saves.get_mut(&path) else {
                continue;
            };
            let snapshot_dir = base
                .join("opened")
                .join(format!("{}-{}", std::process::id(), save.id));
            if let Some(request) = save.begin(path, edits, snapshot_dir, save.force) {
                requests.push(request);
            }
        }
        requests
    }
    /// What the sidebar panel lists for the open tab.
    pub(super) fn sidebar_list(&self) -> SidebarList<'_> {
        let Some(v) = &self.pdf else {
            return SidebarList::Message(if self.frame.is_some() { "Thumbnails, contents, and notes are for PDFs." } else { "" });
        };
        match self.sidebar_tab {
            0 => SidebarList::Thumbnails(&v.sizes),
            1 => match &v.outline {
                Some(Ok(items)) if !items.is_empty() => SidebarList::Contents(items),
                Some(Ok(_)) => SidebarList::Message("No contents"),
                Some(Err(error)) => SidebarList::Message(error),
                None => SidebarList::Message("Loading contents..."),
            },
            _ if !v.notes.is_empty() => SidebarList::Notes(&v.notes),
            _ if v.scanned.iter().all(|done| *done) => SidebarList::Message("No notes"),
            _ => SidebarList::Message("Looking for notes..."),
        }
    }
    /// Accessible name of the document view: file name and page.
    pub(super) fn document_label(&self) -> String {
        let name = self.path.as_deref().map(file_name).unwrap_or_default();
        match self.subtitle() {
            s if s.is_empty() => name,
            s => format!("{name}, {}", s.to_lowercase()),
        }
    }
    pub(super) fn layout(&self) -> Layout {
        let titles: Vec<String> = self.tabs.iter().map(|p| file_name(p)).collect();
        let label = self.document_label();
        let sheet = self.sheet.as_ref().map(|sheet| {
            // Message height in epx, wrapped to the card's inner width.
            let inner = (440f32).min(self.size.0 / self.scale - 32.0).max(160.0) - 48.0;
            let message_height = if sheet.message.is_empty() {
                0.0
            } else {
                // Measured with the current fonts (one cache entry) and converted to epx.
                fonts(self.scale, self.text_scale)
                    .map_or(20.0, |f| (measure(&sheet.message, &f.wrap, inner * self.scale).1 / self.scale).ceil())
            };
            SheetView {
                message_height,
                fields: sheet.fields.iter().map(|f| f.label.as_str()).collect(),
                buttons: sheet.buttons.iter().map(String::as_str).collect(),
                controls: &sheet.controls,
            }
        });
        let mut layout = widgets::layout(&widgets::Input {
            width: self.size.0,
            height: self.size.1,
            scale: self.scale,
            text_scale: self.text_scale,
            tabs: &titles,
            active_tab: self.active_tab(),
            maximized: self.maximized,
            has_document: self.path.is_some(),
            title: &label,
            sidebar_open: self.sidebar_visible(),
            sidebar_tab: self.sidebar_tab,
            sidebar_list: self.sidebar_list(),
            sidebar_scroll: self.sidebar_scroll[self.sidebar_tab.min(2)],
            sidebar_active: if self.sidebar_tab == 0 { self.page as usize } else { self.sidebar_rows[self.sidebar_tab.min(2)] },
            markup: ease(self.markup_progress()),
            ctx: self.ctx(),
            sheet,
        });
        super::empty::add(self, &mut layout);
        super::infobar::add(self, &mut layout);
        super::findbar::add(self, &mut layout);
        layout
    }
    pub(super) fn send(&mut self, job: Job) -> bool {
        self.workers.send(job)
    }
    pub(super) fn request_text_layer(&mut self, page: u32) {
        if self.text_layers.contains_key(&page) || self.text_failed.contains_key(&page) || !self.text_requested.insert(page) {
            return;
        }
        let Some(path) = self.path.clone() else {
            self.text_requested.remove(&page);
            return;
        };
        let request = Request {
            generation: self.generation,
            path,
            page,
            delta: 0,
            width: 1,
            height: 1,
            sessions: self.sessions.clone(),
            sources: self.opened_sources(),
        };
        if !self.send(Job::Layer(request)) {
            self.text_requested.remove(&page);
        }
    }
    fn finish_text_layer(&mut self, page: u32, result: Result<TextLayer, String>) -> Result<(), String> {
        self.text_requested.remove(&page);
        match result {
            Ok(layer) => {
                self.text_failed.remove(&page);
                self.text_layers.insert(page, layer);
                Ok(())
            }
            Err(error) => {
                self.text_failed.insert(page, error.clone());
                Err(error)
            }
        }
    }
    pub(super) fn opened_sources(&self) -> HashMap<PathBuf, PathBuf> {
        self.saves
            .iter()
            .filter_map(|(path, save)| save.snapshot.as_ref().map(|snapshot| (path.clone(), snapshot.clone())))
            .collect()
    }
    /// Clears hover, press, and tooltips when the pointer moves to `id`.
    pub(super) fn set_hover(&mut self, id: Option<WidgetId>) -> bool {
        if self.hover == id {
            return false;
        }
        self.hover = id;
        self.hover_since = id.map(|_| Instant::now());
        self.tooltip = None;
        true
    }
}

pub(super) fn add_tabs(state: &mut State, paths: &[PathBuf]) {
    for path in paths {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        if !state.tabs.contains(&path) {
            state.tabs.push(path);
        }
    }
}

pub(super) unsafe fn invalidate(hwnd: HWND) {
    let _ = InvalidateRect(Some(hwnd), None, false);
}

pub(super) unsafe fn schedule(hwnd: HWND, state: &mut State, delta: i32) {
    let Some(path) = state.path.clone() else {
        return;
    };
    let document = state.layout().document;
    if document.width() < 1.0 || document.height() < 1.0 {
        return;
    }
    state.generation = state.generation.wrapping_add(1);
    state.text_layers.clear();
    state.text_requested.clear();
    state.text_failed.clear();
    state.text_selection = None;
    state.text_drag = false;
    state.find_query.clear();
    state.find_hits.clear();
    state.find_index = 0;
    state.pending = true;
    state.render_failed = false;
    state.status = "Opening...".into();
    let (width, height) = document::image_box(state, document);
    let request = Request {
        generation: state.generation,
        path,
        page: state.page,
        delta,
        width,
        height,
        sessions: state.sessions.clone(),
        sources: state.opened_sources(),
    };
    // A PDF opens to its page sizes; tiles follow from the view.
    let job = if is_pdf(&request.path) { Job::Open(request) } else { Job::Render(request) };
    if !state.send(job) {
        state.pending = false;
        state.status = "The rendering worker stopped. Close Preview and reopen the file.".into();
    }
    invalidate(hwnd);
}

pub(super) unsafe fn open(hwnd: HWND, path: PathBuf) {
    with_state(|state| {
        if let Some((path, _)) = &state.displayed {
            if !state.pending && !state.render_failed {
                // The page now in view; a PDF has scrolled since it opened.
                state.views.insert(path.clone(), (state.page, state.zoom, state.pan));
            }
        }
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        let view = state.views.get(&path).copied().unwrap_or((0, Zoom::Fit, (0.0, 0.0)));
        state.path = Some(path);
        state.page = view.0;
        state.due = None;
        state.zoom = view.1;
        state.pan = view.2;
        state.crop = false;
        state.zoom_select = false;
        state.markup = None;
        state.selection = None;
        if !state.is_pdf() && matches!(state.focus, Some(WidgetId::SidebarTab(_) | WidgetId::SidebarItem(_))) {
            state.focus = Some(WidgetId::Document);
        }
        if state.focus.is_none() || matches!(state.focus, Some(WidgetId::Command(Command::Open))) {
            state.focus = Some(WidgetId::Document);
        }
        schedule(hwnd, state, 0);
    });
}

pub(super) unsafe fn navigate(hwnd: HWND, delta: i32) {
    with_state(|state| {
        if state.pending {
            return;
        }
        if state.is_pdf() {
            document::step_page(state, delta);
            invalidate(hwnd);
        } else {
            // The next image opens at fit, so its pre-decoded frame matches.
            state.zoom = Zoom::Fit;
            state.pan = (0.0, 0.0);
            schedule(hwnd, state, delta);
        }
    });
}

pub(super) unsafe fn select_tab(hwnd: HWND, index: usize) {
    let path = with_state(|s| s.tabs.get(index).cloned().filter(|p| s.path.as_ref() != Some(p))).flatten();
    if let Some(path) = path {
        open(hwnd, path);
    }
}

pub(super) unsafe fn close_tab(hwnd: HWND, index: usize) {
    with_state(|s| s.pause_save_dispatch(true));
    let Some((path, dirty, saving)) = with_state(|s| {
        s.tabs.get(index).cloned().map(|p| {
            let (dirty, saving) = s.tab_close_state(&p);
            (p, dirty, saving)
        })
    })
    .flatten()
    else {
        with_state(|s| s.pause_save_dispatch(false));
        return;
    };
    if saving {
        sheet::alert(hwnd, "Saving", "Wait for this file to finish saving, then close the tab.");
        with_state(|s| s.pause_save_dispatch(false));
        return;
    }
    if dirty
        && !sheet::confirm(
            hwnd,
            "Close this tab?",
            &format!("Some edits to {} are not saved. Closing now discards those edits.", file_name(&path)),
            "Close without saving",
        )
    {
        with_state(|s| s.pause_save_dispatch(false));
        return;
    }
    if with_state(|s| s.tab_close_state(&path).1).unwrap_or(true) {
        sheet::alert(hwnd, "Saving", "This file started saving. Wait for it to finish, then close the tab.");
        with_state(|s| s.pause_save_dispatch(false));
        return;
    }
    let next = with_state(|s| {
        s.tabs.retain(|p| *p != path);
        s.sessions.remove(&path);
        if let Some(snapshot) = s.saves.remove(&path).and_then(|save| save.snapshot) {
            if let Some(folder) = snapshot.parent() {
                let _ = std::fs::remove_dir_all(folder);
            }
        }
        s.views.remove(&path);
        s.password_attempts.remove(&path);
        s.pause_save_dispatch(false);
        if s.path.as_ref() != Some(&path) {
            return None;
        }
        let next = s.tabs.get(index).or_else(|| s.tabs.get(index.wrapping_sub(1))).cloned();
        if next.is_none() {
            // Last tab: back to the empty window. Results in flight are dropped.
            s.generation = s.generation.wrapping_add(1);
            s.path = None;
            s.frame = None;
            s.pdf = None;
            s.displayed = None;
            s.pending = false;
            s.render_failed = false;
            s.crop = false;
            s.markup = None;
            s.signature = None;
            s.slideshow = None;
            s.selection = None;
            s.focus = None;
            s.status = EMPTY_STATUS.into();
            s.recent = super::empty::load();
            if let Some(r) = s.renderer.as_mut() {
                r.bitmap = None;
            }
        }
        next
    })
    .flatten();
    match next {
        Some(next) => open(hwnd, next),
        None => {
            let _ = SetWindowTextW(hwnd, windows::core::w!("Preview for Windows"));
            invalidate(hwnd);
        }
    }
}

fn current_text_result(current_generation: u64, current_path: Option<&std::path::Path>, generation: u64, path: &std::path::Path) -> bool {
    generation == current_generation && current_path == Some(path)
}

/// Drains worker results and runs timed work. Runs on WM_TIMER and on the
/// worker's wake message.
pub(super) unsafe fn tick(hwnd: HWND) {
    let mut fields_to_show = None;
    let mut output_to_open = None;
    let mut advance = false;
    let mut password_to_show = None;
    let mut start_services = false;
    let mut title = None;
    let mut info_to_show = None;
    with_state(|state| {
        while let Ok(event) = state.workers.receiver.try_recv() {
            let completed = match event {
                Event::Render(completed) => completed,
                Event::Pages(opened) => {
                    if opened.generation != state.generation {
                        continue;
                    }
                    state.pending = false;
                    match opened.result {
                        Ok(sizes) => {
                            if let Some(stamp) = opened.opened {
                                state.opened_for_save(opened.path.clone(), stamp);
                            }
                            title = Some(shown(state, opened.path.clone(), opened.page, false));
                            state.page = opened.page.min(sizes.len() as u32 - 1);
                            let same = state.pdf.as_ref().is_some_and(|v| v.path == opened.path);
                            match state.pdf.as_mut().filter(|_| same) {
                                Some(v) => {
                                    if v.update(sizes, opened.edits, &mut state.cache) {
                                        state.sent.clear();
                                    }
                                    v.reveal = Some(state.page);
                                }
                                None => {
                                    state.pdf = Some(PdfView::new(opened.path, sizes, opened.edits, state.page));
                                    state.frame = None;
                                    if let Some(renderer) = state.renderer.as_mut() {
                                        renderer.bitmap = None;
                                    }
                                }
                            }
                            state.status = state.subtitle();
                            state.request_text_layer(state.page);
                        }
                        Err(error) => {
                            if let Some(prompt) = failed(state, error, opened.generation, opened.path, opened.page) {
                                password_to_show = Some(prompt);
                            }
                        }
                    }
                    invalidate(hwnd);
                    continue;
                }
                Event::Done(item, outcome) => {
                    state.workers.delivered(&item.key);
                    received(state, item, outcome);
                    invalidate(hwnd);
                    continue;
                }
                Event::Metadata(generation, result) => {
                    if generation == state.generation {
                        info_to_show = Some(result);
                    }
                    continue;
                }
                Event::Saved(path, edits, whole_document, result) => {
                    state.exporting = false;
                    let result = result.map(|()| {
                        let autosave_managed = state.saves.get(&path).is_some_and(|save| save.target.is_some());
                        if let Some(current) = state.sessions.get_mut(&path) {
                            if !autosave_managed && whole_document && current.image == edits.image && current.pdf == edits.pdf {
                                current.dirty = false;
                            }
                        }
                        "Saved a new copy. The original file is unchanged.".to_string()
                    });
                    super::infobar::report(state, result);
                    invalidate(hwnd);
                    continue;
                }
                Event::DragOut(output, result) => {
                    super::organize::extracted(state, output, result);
                    continue;
                }
                Event::Autosaved(saved) => {
                    if state.autosaved(saved) {
                        invalidate(hwnd);
                    }
                    continue;
                }
                Event::Layer(generation, path, page, result) => {
                    if !current_text_result(state.generation, state.path.as_deref(), generation, &path) {
                        continue;
                    }
                    if let Err(error) = state.finish_text_layer(page, result) {
                        state.status = error;
                    }
                    invalidate(hwnd);
                    continue;
                }
                Event::Text(generation, path, result) => {
                    if !current_text_result(state.generation, state.path.as_deref(), generation, &path) || state.pending {
                        continue;
                    }
                    let result = match result {
                        Ok(text) if text.is_empty() => Ok("No text found.".to_string()),
                        Ok(text) => super::files::clipboard(hwnd, &super::text::clipboard_text(&text))
                            .map(|()| "Text copied. Check recognition and reading order before pasting.".to_string()),
                        Err(error) => Err(error),
                    };
                    super::infobar::report(state, result);
                    invalidate(hwnd);
                    continue;
                }
                Event::Found(generation, path, result) => {
                    if super::findbar::found(state, generation, path, result) {
                        invalidate(hwnd);
                    }
                    continue;
                }
                Event::Fields(generation, result) => {
                    if generation == state.generation {
                        match result {
                            Ok(fields) => fields_to_show = Some((generation, fields)),
                            Err(error) => state.status = error,
                        }
                        invalidate(hwnd);
                    }
                    continue;
                }
                Event::Background(output, result) => {
                    state.exporting = false;
                    if result.is_ok() {
                        output_to_open = Some(output);
                    }
                    super::infobar::report(state, result);
                    invalidate(hwnd);
                    continue;
                }
                Event::Finished(result) => {
                    state.exporting = false;
                    state.cancel = None;
                    super::infobar::report(state, result);
                    invalidate(hwnd);
                    continue;
                }
                Event::Progress(text) => {
                    state.status = text;
                    invalidate(hwnd);
                    continue;
                }
                Event::Tool(done) => {
                    super::imagetools::received(hwnd, state, done);
                    invalidate(hwnd);
                    continue;
                }
            };
            if completed.generation != state.generation {
                continue;
            }
            state.pending = false;
            match completed.result {
                Ok(frame) => {
                    if crate::model::frame_bytes(frame.width, frame.height).ok() != Some(frame.pixels.len()) {
                        state.status = "The decoder returned an invalid image.".into();
                        continue;
                    }
                    if let Some(stamp) = completed.opened {
                        state.opened_for_save(completed.path.clone(), stamp);
                    }
                    title = Some(shown(state, completed.path, completed.page, completed.navigation));
                    state.frame = Some(frame);
                    state.pdf = None;
                    state.from_predecode = completed.from_predecode;
                    state.status = state.subtitle();
                    if let Some(renderer) = state.renderer.as_mut() {
                        renderer.bitmap = None;
                    }
                }
                Err(error) => {
                    if let Some(prompt) = failed(state, error, completed.generation, completed.path, completed.page) {
                        password_to_show = Some(prompt);
                    }
                }
            }
            invalidate(hwnd);
        }
        if state.painted && state.due.is_some_and(|due| Instant::now() >= due) {
            state.due = None;
            // A PDF on screen only renders tiles at the new scale; anything
            // else (first open, an image at a new size) loads again.
            let showing = state.pdf.as_ref().is_some_and(|v| Some(&v.path) == state.path.as_ref());
            if state.pending {
                // Try again once the file in flight arrives.
                state.due = Some(Instant::now() + document::SETTLE);
            } else if !showing {
                schedule(hwnd, state, 0);
            } else if document::settle(state) {
                invalidate(hwnd);
            }
        }
        for request in state.autosave_requests(Instant::now()) {
            if !state.send(Job::Autosave(request.clone())) {
                let failed = Autosaved {
                    path: request.path,
                    session_id: request.session_id,
                    revision: request.revision,
                    snapshot: request.snapshot,
                    target: request.target,
                    result: Err(super::disk::Failure::Error("The saving worker stopped.".into())),
                };
                state.autosaved(failed);
            }
            invalidate(hwnd);
        }
        if !state.pending && state.slideshow.is_some_and(|due| Instant::now() >= due) {
            if state.is_pdf() && state.page + 1 >= state.page_count() {
                state.slideshow = None;
                state.status = "End of slideshow.".into();
            } else {
                state.slideshow = Some(Instant::now() + Duration::from_secs(3));
                advance = true;
            }
        }
        if let Some(since) = state.markup_since {
            if !state.animations || since.elapsed() >= SLIDE {
                state.markup_since = None;
                // The document area changed size; render at the new size.
                if state.frame.is_some() {
                    state.due = Some(Instant::now());
                }
            }
            invalidate(hwnd);
        }
        if super::organize::autoscroll(state) {
            invalidate(hwnd);
        }
        if state.tooltip.is_none() && state.hover_since.is_some_and(|t| t.elapsed() >= TOOLTIP_DELAY) {
            state.hover_since = None;
            state.tooltip = state.hover;
            invalidate(hwnd);
        }
        if super::findbar::tick(state) {
            invalidate(hwnd);
        }
        if super::infobar::tick(state) {
            invalidate(hwnd);
        }
        // Accessibility (UI Automation load) and UISettings (12 to 23 ms to
        // create on this PC) wait until the first content is on screen, so
        // they never delay it.
        let first_content = state.content_drawn || state.render_failed || state.path.is_none();
        if !state.started && state.painted && first_content && !state.pending {
            state.started = true;
            start_services = true;
        }
    });
    if let Some(title) = title {
        let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
        let _ = SetWindowTextW(hwnd, PCWSTR(title.as_ptr()));
    }
    super::findbar::place(hwnd);
    if start_services {
        super::a11y::start(hwnd);
        super::window::watch_text_scale(hwnd);
        super::empty::prune(hwnd);
        super::infobar::after_launch(hwnd);
    }
    if let Some((generation, fields)) = fields_to_show {
        super::actions::fill_field(hwnd, generation, fields);
    }
    if let Some(result) = info_to_show {
        super::actions::pdf_info(hwnd, result);
    }
    super::bench::tick(hwnd);
    super::organize::after_tick(hwnd);
    if let Some(output) = output_to_open {
        open(hwnd, output);
    }
    if let Some((generation, path, page)) = password_to_show {
        password_prompt(hwnd, generation, path, page);
    }
    if advance {
        navigate(hwnd, 1);
    }
    super::imagetools::tick(hwnd);
}

/// A file is on screen: tabs, title, and history follow it. Next or
/// previous image replaces the tab, unless that tab holds unsaved edits.
/// Returns the window title.
fn shown(state: &mut State, path: PathBuf, page: u32, navigation: bool) -> String {
    let previous = state.displayed.as_ref().map(|(p, _)| p.clone());
    let previous_dirty = previous.as_ref().and_then(|p| state.sessions.get(p)).is_some_and(|edits| edits.dirty);
    if navigation {
        if let Some((old, page)) = &state.displayed {
            if old != &path {
                state.views.insert(old.clone(), (*page, state.zoom, state.pan));
            }
        }
    }
    state.password_attempts.remove(&path);
    if !navigation && previous.as_ref() != Some(&path) {
        super::empty::note(&path);
    }
    state.displayed = Some((path.clone(), page));
    if navigation && !previous_dirty && !state.tabs.contains(&path) {
        if let Some(index) = previous.and_then(|p| state.tabs.iter().position(|t| *t == p)) {
            state.tabs[index] = path.clone();
        }
    }
    if !state.tabs.contains(&path) {
        state.tabs.push(path.clone());
    }
    let title = format!("{} - Preview for Windows", file_name(&path));
    state.path = Some(path);
    state.page = page;
    state.render_failed = false;
    title
}

/// A file could not open; the previous view stays. Returns the password
/// prompt to show, if the PDF needs one.
fn failed(state: &mut State, error: String, generation: u64, path: PathBuf, page: u32) -> Option<(u64, PathBuf, u32)> {
    state.render_failed = error != "No more images in this direction.";
    if state.render_failed && error != crate::pdf::PASSWORD_REQUIRED {
        super::infobar::error(state, format!("Could not open {}. {error}", file_name(&path)));
    }
    state.status = format!("{error} Previous view retained.");
    state.slideshow = None;
    let prompt = (error == crate::pdf::PASSWORD_REQUIRED).then(|| {
        state.due = None;
        (generation, path, page)
    });
    if let Some((path, page)) = &state.displayed {
        if state.path.as_ref() != Some(path) {
            if let Some((_, zoom, pan)) = state.views.get(path) {
                state.zoom = *zoom;
                state.pan = *pan;
            }
        }
        state.path = Some(path.clone());
        state.page = *page;
    }
    prompt
}

/// A tile, thumbnail, outline, or page of notes from the document worker.
/// Results made for older edits are kept as stale tiles and asked for again.
fn received(state: &mut State, item: super::worker::Item, outcome: Outcome) {
    // Image pre-decodes stay in the worker cache. Their completion does not
    // invalidate the view's work list, or every repaint queues them again.
    if matches!(outcome, Outcome::Predecoded) {
        return;
    }
    let fresh = state.pdf.as_ref().is_some_and(|v| v.doc == item.key.doc && *v.edits == *item.edits);
    if state.pdf.as_ref().is_some_and(|v| v.doc == item.key.doc) && !fresh {
        state.sent.clear();
    }
    match outcome {
        Outcome::Frame(Ok(frame)) if crate::model::frame_bytes(frame.width, frame.height).ok() == Some(frame.pixels.len()) => {
            state.arrived.push((item.key, frame, fresh));
        }
        Outcome::Frame(_) => {
            if let Some(v) = state.pdf.as_mut().filter(|v| v.doc == item.key.doc) {
                v.failed.insert(item.key);
            }
        }
        Outcome::Outline(result) if fresh => {
            if let Some(v) = state.pdf.as_mut() {
                v.outline = Some(result);
            }
        }
        Outcome::Notes(result) if fresh => {
            if let (Some(v), Work::Notes { page }) = (state.pdf.as_mut(), item.key.work) {
                if let Some(done) = v.scanned.get_mut(page as usize) {
                    *done = true;
                }
                let notes = result.unwrap_or_default();
                let at = v.notes.partition_point(|n| n.page <= page);
                v.notes.splice(at..at, notes);
            }
        }
        Outcome::Outline(_) | Outcome::Notes(_) | Outcome::Predecoded => {}
    }
}

unsafe fn password_prompt(hwnd: HWND, generation: u64, path: PathBuf, page: u32) {
    let retry = with_state(|s| s.password_attempts.get(&path).copied().unwrap_or(0) > 0).unwrap_or(false);
    let message = if retry {
        "That password did not unlock this PDF. Try again.".to_string()
    } else {
        format!("{} is protected. Enter its password to open it.", file_name(&path))
    };
    let Some(password) = sheet::password(hwnd, "Open protected PDF", &message) else {
        with_state(|state| {
            state.password_attempts.remove(&path);
            state.render_failed = state.frame.is_none();
            state.status = "PDF opening cancelled. The previous document is unchanged.".into();
        });
        invalidate(hwnd);
        return;
    };
    with_state(|state| {
        if state.generation != generation {
            return;
        }
        *state.password_attempts.entry(path.clone()).or_default() += 1;
        state.generation = state.generation.wrapping_add(1);
        state.pending = true;
        state.render_failed = false;
        state.path = Some(path.clone());
        state.page = page;
        state.status = "Unlocking PDF...".into();
        let request = Request {
            generation: state.generation,
            path,
            page,
            delta: 0,
            width: 1,
            height: 1,
            sessions: state.sessions.clone(),
            sources: state.opened_sources(),
        };
        if !state.send(Job::Password(request, password)) {
            state.pending = false;
            state.status = "The PDF worker stopped.".into();
        }
    });
    invalidate(hwnd);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_results_require_the_current_generation_and_path() {
        let current = std::path::Path::new("current.pdf");
        assert!(current_text_result(7, Some(current), 7, current));
        assert!(!current_text_result(7, Some(current), 6, current));
        assert!(!current_text_result(7, Some(current), 7, std::path::Path::new("other.pdf")));
        assert!(!current_text_result(7, None, 7, current));
    }

    #[test]
    fn terminal_text_layer_failures_do_not_requeue_until_a_new_generation() {
        use super::super::theme::{palette, Mode};
        for error in ["This PDF's permissions do not allow reading its text.", "The PDF page is corrupt."] {
            let workers = Workers::start(HWND::default()).unwrap();
            let path = PathBuf::from("report.pdf");
            let mut state = State::new(workers, &[path], (800.0, 600.0), 1.0, 1.0, palette(Mode::Light));
            state.generation = 7;
            state.text_requested.insert(3);
            assert_eq!(state.finish_text_layer(3, Err(error.into())), Err(error.into()));
            assert!(!state.text_requested.contains(&3));
            state.request_text_layer(3);
            assert!(!state.text_requested.contains(&3), "paint must not retry a terminal page failure");
            state.generation += 1;
            state.text_failed.clear();
            state.request_text_layer(3);
            assert!(state.text_requested.contains(&3), "an explicit reopen may retry the page");
            state.workers.stop();
        }
    }

    fn stamp(len: u64) -> super::super::disk::Stamp {
        super::super::disk::Stamp {
            len,
            modified: Some(std::time::SystemTime::UNIX_EPOCH),
            created: Some(std::time::SystemTime::UNIX_EPOCH),
        }
    }

    #[test]
    fn save_completion_keeps_an_edit_made_while_saving_dirty() {
        let path = PathBuf::from("report.pdf");
        let target = path.clone();
        let mut save = SaveSession::new(7, stamp(10));
        save.configure(target.clone(), Some(stamp(10)));
        save.edited(Instant::now());
        let request = save
            .begin(path.clone(), Edits::default(), PathBuf::from("snapshot"), false)
            .unwrap();
        assert_eq!(save.snapshot, Some(PathBuf::from("snapshot/report.pdf")));
        assert_eq!(request.snapshot, None, "the worker still creates the first immutable copy");
        save.edited(Instant::now());
        let event = Autosaved {
            path,
            session_id: request.session_id,
            revision: request.revision,
            snapshot: Some(PathBuf::from("snapshot/report.pdf")),
            target,
            result: Ok(stamp(20)),
        };
        assert_eq!(save.apply(&event), Some(false));
        assert_eq!((save.saved_revision, save.revision), (1, 2));
        assert_eq!(save.expected, Some(stamp(20)), "the next save compares against the version just written");
        assert!(save.due.is_some());
        assert_eq!(save.status, SaveStatus::Edited);
    }

    #[test]
    fn stale_save_events_and_conflicts_never_clear_recipes() {
        use super::super::theme::{palette, Mode};
        let workers = Workers::start(HWND::default()).unwrap();
        let path = PathBuf::from("photo.png");
        let mut state = State::new(workers, &[path.clone()], (800.0, 600.0), 1.0, 1.0, palette(Mode::Light));
        state.sessions.insert(
            path.clone(),
            Edits { image: vec![crate::model::ImageEdit::RotateRight], pdf: Vec::new(), dirty: true },
        );
        state.saves.insert(path.clone(), SaveSession::new(11, stamp(4)));
        let save = state.saves.get_mut(&path).unwrap();
        save.configure(path.clone(), Some(stamp(4)));
        save.edited(Instant::now());
        save.in_flight = Some(1);
        let event = |session_id, revision, result| Autosaved {
            path: path.clone(),
            session_id,
            revision,
            snapshot: None,
            target: path.clone(),
            result,
        };
        assert!(!state.autosaved(event(10, 1, Ok(stamp(5)))), "closed session result is stale");
        assert!(!state.autosaved(event(11, 0, Ok(stamp(5)))), "old revision result is stale");
        assert!(state.sessions[&path].dirty);
        assert!(state.autosaved(event(11, 1, Err(super::super::disk::Failure::Changed))));
        assert!(state.sessions[&path].dirty);
        assert_eq!(state.saves[&path].status, SaveStatus::Conflict);
        assert_eq!(state.sessions[&path].image.len(), 1, "conflict keeps the recipe");
        state.workers.stop();
    }

    #[test]
    fn inactive_tab_save_completion_does_not_replace_visible_status() {
        use super::super::theme::{palette, Mode};
        let workers = Workers::start(HWND::default()).unwrap();
        let active = PathBuf::from("active.pdf");
        let background = PathBuf::from("background.pdf");
        let mut state = State::new(
            workers,
            &[active.clone(), background.clone()],
            (800.0, 600.0),
            1.0,
            1.0,
            palette(Mode::Light),
        );
        state.path = Some(active);
        state.status = "Active document status".into();
        state.sessions.insert(background.clone(), Edits { dirty: true, ..Edits::default() });
        let mut save = SaveSession::new(12, stamp(4));
        save.configure(background.clone(), Some(stamp(4)));
        save.edited(Instant::now());
        save.in_flight = Some(1);
        state.saves.insert(background.clone(), save);
        assert!(state.autosaved(Autosaved {
            path: background.clone(),
            session_id: 12,
            revision: 1,
            snapshot: Some(PathBuf::from("opened/background.pdf")),
            target: background.clone(),
            result: Ok(stamp(5)),
        }));
        assert_eq!(state.status, "Active document status");
        assert!(!state.sessions[&background].dirty, "the background tab still records its successful save");
        assert_eq!(state.saves[&background].status, SaveStatus::Saved);
        state.workers.stop();
    }

    #[test]
    fn due_edits_schedule_one_document_worker_save() {
        use super::super::theme::{palette, Mode};
        let workers = Workers::start(HWND::default()).unwrap();
        let path = PathBuf::from("photo.png");
        let mut state = State::new(workers, &[path.clone()], (800.0, 600.0), 1.0, 1.0, palette(Mode::Light));
        state.sessions.insert(
            path.clone(),
            Edits { image: vec![crate::model::ImageEdit::RotateRight], pdf: Vec::new(), dirty: true },
        );
        let mut save = SaveSession::new(20, stamp(10));
        save.configure(path.clone(), Some(stamp(10)));
        save.edited(Instant::now());
        save.due = Some(Instant::now() - Duration::from_millis(1));
        state.saves.insert(path.clone(), save);
        state.pause_save_dispatch(true);
        assert!(state.autosave_requests(Instant::now()).is_empty(), "a modal close barrier pauses dispatch");
        state.pause_save_dispatch(false);
        let requests = state.autosave_requests(Instant::now());
        assert_eq!(requests.len(), 1);
        assert_eq!((requests[0].session_id, requests[0].revision), (20, 1));
        assert_eq!(requests[0].edits.image.len(), 1);
        assert!(state.autosave_requests(Instant::now()).is_empty(), "an in-flight revision is never queued twice");
        assert_eq!(state.saves[&path].status, SaveStatus::Saving);
        state.workers.stop();
    }

    #[test]
    fn revert_to_opened_schedules_an_empty_recipe() {
        use super::super::theme::{palette, Mode};
        let workers = Workers::start(HWND::default()).unwrap();
        let path = PathBuf::from("report.pdf");
        let mut state = State::new(workers, &[path.clone()], (800.0, 600.0), 1.0, 1.0, palette(Mode::Light));
        state.sessions.insert(path.clone(), Edits { dirty: true, ..Edits::default() });
        let mut save = SaveSession::new(21, stamp(10));
        save.configure(path.clone(), Some(stamp(12)));
        save.revision = 1;
        save.saved_revision = 1;
        save.status = SaveStatus::Saved;
        save.edited(Instant::now());
        save.due = Some(Instant::now() - Duration::from_millis(1));
        state.saves.insert(path, save);
        let requests = state.autosave_requests(Instant::now());
        assert_eq!(requests.len(), 1);
        assert!(requests[0].edits.image.is_empty() && requests[0].edits.pdf.is_empty());
        assert_eq!(requests[0].expected, Some(stamp(12)), "revert replaces only the version Preview last saved");
        state.workers.stop();
    }

    #[test]
    fn close_state_blocks_in_flight_saves_and_marks_unsaved_tabs() {
        use super::super::theme::{palette, Mode};
        let workers = Workers::start(HWND::default()).unwrap();
        let path = PathBuf::from("report.pdf");
        let mut state = State::new(workers, &[path.clone()], (800.0, 600.0), 1.0, 1.0, palette(Mode::Light));
        state.sessions.insert(path.clone(), Edits { dirty: true, ..Edits::default() });
        let mut save = SaveSession::new(22, stamp(10));
        save.configure(path.clone(), Some(stamp(10)));
        save.edited(Instant::now());
        save.in_flight = Some(1);
        state.saves.insert(path.clone(), save);
        assert_eq!(state.tab_close_state(&path), (true, true));
        assert_eq!(state.window_close_state(), (true, true));
        state.sessions.get_mut(&path).unwrap().dirty = false;
        assert_eq!(
            state.tab_close_state(&path),
            (true, true),
            "a legacy Save a copy result cannot hide an unsaved autosave revision"
        );
        state.workers.stop();
    }

    #[test]
    fn close_state_keeps_a_snapshot_while_a_non_save_job_reads_it() {
        use super::super::theme::{palette, Mode};
        let workers = Workers::start(HWND::default()).unwrap();
        let path = PathBuf::from("report.pdf");
        let mut state = State::new(workers, &[path.clone()], (800.0, 600.0), 1.0, 1.0, palette(Mode::Light));
        let mut save = SaveSession::new(24, stamp(10));
        save.snapshot = Some(PathBuf::from("opened/report.pdf"));
        state.saves.insert(path.clone(), save);
        let mut sources = HashMap::new();
        sources.insert(path.clone(), PathBuf::from("opened/report.pdf"));
        state.workers.retain_test_job_readers(&Job::Metadata(Request {
            generation: 1,
            path: path.clone(),
            page: 0,
            delta: 0,
            width: 1,
            height: 1,
            sessions: HashMap::new(),
            sources,
        }));
        assert_eq!(state.tab_close_state(&path), (false, true), "tab cleanup waits for a snapshot reader");
        assert_eq!(state.window_close_state(), (false, true), "shutdown cleanup waits for any snapshot reader");
        state.workers.stop();
    }

    #[test]
    fn close_state_keeps_snapshot_for_queued_and_active_view_work() {
        use super::super::{theme::{palette, Mode}, worker::{Item, Key, Work}};
        let workers = Workers::start(HWND::default()).unwrap();
        let path = PathBuf::from("report.pdf");
        let snapshot = PathBuf::from("opened/report.pdf");
        let mut state = State::new(workers, &[path.clone()], (800.0, 600.0), 1.0, 1.0, palette(Mode::Light));
        let mut save = SaveSession::new(25, stamp(10));
        save.snapshot = Some(snapshot.clone());
        state.saves.insert(path.clone(), save);
        let key = Key { doc: super::super::worker::doc_id(&path), work: Work::Thumb { page: 0 } };
        let item = Item { key, path: snapshot.clone(), edits: Arc::default(), scale: 1.0, region: [0, 0, 100, 100] };
        let sources = HashMap::from([(path.clone(), snapshot)]);
        state.workers.retain_test_view_items(vec![item], &sources);
        assert_eq!(state.tab_close_state(&path), (false, true), "queued view work leases its snapshot");
        assert!(state.workers.take_test_view_item().is_some());
        assert_eq!(state.tab_close_state(&path), (false, true), "active view work keeps the lease");
        state.workers.delivered(&key);
        assert_eq!(state.tab_close_state(&path), (false, false), "delivery releases the view lease");
        state.workers.stop();
    }

    #[test]
    fn delta_navigation_keeps_a_sibling_snapshot_until_the_read_finishes() {
        use super::super::theme::{palette, Mode};
        let workers = Workers::start(HWND::default()).unwrap();
        let first = PathBuf::from("001.png");
        let sibling = PathBuf::from("002.png");
        let state = State::new(
            workers,
            &[first.clone(), sibling.clone()],
            (800.0, 600.0),
            1.0,
            1.0,
            palette(Mode::Light),
        );
        let mut sources = HashMap::new();
        sources.insert(first.clone(), PathBuf::from("opened/001.png"));
        sources.insert(sibling.clone(), PathBuf::from("opened/002.png"));
        let job = Job::Render(Request {
            generation: 1,
            path: first,
            page: 0,
            delta: 1,
            width: 100,
            height: 100,
            sessions: HashMap::new(),
            sources,
        });
        state.workers.retain_test_job_readers(&job);
        assert_eq!(
            state.tab_close_state(&sibling),
            (false, true),
            "closing the sibling waits while delta navigation may read its snapshot"
        );
        state.workers.stop();
    }

    #[test]
    fn conflict_resolution_restarts_with_the_explicit_target_policy() {
        let mut save = SaveSession::new(23, stamp(10));
        save.configure(PathBuf::from("report.pdf"), Some(stamp(10)));
        save.status = SaveStatus::Conflict;
        save.resolve_conflict(PathBuf::from("report (edited).pdf"), None, false);
        let request = save
            .begin(
                PathBuf::from("report.pdf"),
                Edits { pdf: vec![crate::model::PdfEdit::RotateRight { page: 0 }], ..Edits::default() },
                PathBuf::from("opened"),
                save.force,
            )
            .unwrap();
        assert_eq!(request.target, PathBuf::from("report (edited).pdf"));
        assert_eq!(request.expected, None);
        assert!(!request.force);
    }

    #[test]
    fn save_consent_buttons_fit_at_225_percent_text() {
        use super::super::theme::{palette, Mode};
        let workers = Workers::start(HWND::default()).unwrap();
        let mut state = State::new(workers, &[], (900.0, 700.0), 1.0, 2.25, palette(Mode::Light));
        let fonts = fonts(1.0, 2.25).unwrap();
        for labels in [["Overwrite", "Save copy"], ["Remember", "This file only"]] {
            state.sheet = Some(Sheet {
                title: "Save edits".into(),
                message: "Choose how Preview should save this file.".into(),
                fields: Vec::new(),
                buttons: labels.iter().map(|label| (*label).into()).collect(),
                cancel: usize::MAX,
                result: None,
            });
            let layout = state.layout();
            let buttons: Vec<_> = layout
                .widgets
                .iter()
                .filter(|widget| matches!(widget.id, WidgetId::SheetButton(_)))
                .collect();
            assert_eq!(buttons.len(), 2);
            for (button, label) in buttons.iter().zip(labels) {
                let text_width = measure(label, &fonts.body, 1000.0).0;
                assert!(
                    text_width + 8.0 <= button.rect.width(),
                    "{label:?} is {text_width}px wide but its button is only {}px",
                    button.rect.width()
                );
            }
        }
        state.workers.stop();
    }

    #[test]
    fn image_tabs_hide_the_pdf_sidebar_without_losing_its_preference() {
        use super::super::theme::{palette, Mode};
        let workers = Workers::start(HWND::default()).unwrap();
        let mut state = State::new(workers, &[PathBuf::from("photo.png")], (1000.0, 800.0), 1.0, 1.0, palette(Mode::Light));
        state.sidebar_open = true;
        assert!(!state.sidebar_visible());
        let image_layout = state.layout();
        assert!(image_layout.sidebar.is_none());
        assert!(!image_layout.widgets.iter().find(|w| w.id == WidgetId::Command(Command::ToggleSidebar)).unwrap().enabled);
        assert!(!state.ctx().sidebar_open);
        state.path = Some(PathBuf::from("report.pdf"));
        assert!(state.sidebar_visible());
        assert!(state.layout().sidebar.is_some());
        assert!(state.sidebar_open, "the PDF preference survives the image tab");
        state.workers.stop();
    }

    #[test]
    fn predecode_and_other_document_results_do_not_invalidate_sent_work() {
        use super::super::{theme::{palette, Mode}, worker::{doc_id, Item}};
        let workers = Workers::start(HWND::default()).unwrap();
        let path = PathBuf::from("current.png");
        let mut state = State::new(workers, &[path.clone()], (1000.0, 900.0), 1.0, 1.0, palette(Mode::Light));
        let neighbor = Item {
            key: Key { doc: doc_id(&path), work: Work::Predecode { delta: 1, width: 1000, height: 700 } },
            path,
            edits: Arc::default(),
            scale: 0.0,
            region: [0; 4],
        };
        state.sent = vec![neighbor.key];
        for _ in 0..3 {
            received(&mut state, neighbor.clone(), Outcome::Predecoded);
            assert_eq!(state.sent, vec![neighbor.key], "completion must not trigger another neighbor request");
        }
        let current = PathBuf::from("current.pdf");
        state.pdf = Some(PdfView::new(current.clone(), vec![[612.0, 792.0]], Arc::default(), 0));
        received(&mut state, neighbor.clone(), Outcome::Predecoded);
        assert_eq!(state.sent, vec![neighbor.key], "an old image result must not reset PDF work");
        let mut outline = neighbor.clone();
        outline.key.work = Work::Outline;
        received(&mut state, outline.clone(), Outcome::Outline(Ok(Vec::new())));
        assert_eq!(state.sent, vec![neighbor.key], "another document must not reset this document's work");
        outline.key.doc = doc_id(&current);
        outline.edits = Arc::new(vec![crate::model::PdfEdit::RotateRight { page: 0 }]);
        received(&mut state, outline, Outcome::Outline(Ok(Vec::new())));
        assert!(state.sent.is_empty(), "stale edits of the current PDF still need replacement work");
        state.workers.stop();
    }

    #[test]
    fn markup_bar_slides_over_167_ms_unless_animations_are_off() {
        let ms = Duration::from_millis;
        assert_eq!(slide_progress(0.0, true, ms(0), true), 0.0);
        assert!((slide_progress(0.0, true, ms(83), true) - 0.497).abs() < 0.01);
        assert_eq!(slide_progress(0.0, true, ms(400), true), 1.0);
        assert_eq!(slide_progress(1.0, false, ms(167), true), 0.0);
        assert_eq!(slide_progress(0.4, true, ms(0), false), 1.0, "no animation jumps to the end");
        assert!((slide_progress(0.5, false, ms(0), true) - 0.5).abs() < 1e-6, "a reversed slide starts where it was");
        assert_eq!((ease(0.0), ease(1.0)), (0.0, 1.0));
        assert!(ease(0.5) > 0.5, "decelerates");
    }
}
