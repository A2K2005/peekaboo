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
    worker::{is_pdf, Edits, Event, Job, Key, Outcome, Request, Work, Workers},
};
use crate::model::{AnnotationKind, Frame};
use std::{
    cell::RefCell,
    collections::HashMap,
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
        };
        add_tabs(&mut state, paths);
        state
    }
    pub(super) fn is_pdf(&self) -> bool {
        self.path.as_ref().is_some_and(|p| is_pdf(p))
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
            sidebar_open: self.sidebar_open,
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
            }
        });
        widgets::layout(&widgets::Input {
            width: self.size.0,
            height: self.size.1,
            scale: self.scale,
            text_scale: self.text_scale,
            tabs: &titles,
            active_tab: self.active_tab(),
            maximized: self.maximized,
            has_document: self.path.is_some(),
            title: &label,
            sidebar_open: self.sidebar_open,
            sidebar_tab: self.sidebar_tab,
            sidebar_list: self.sidebar_list(),
            sidebar_scroll: self.sidebar_scroll[self.sidebar_tab.min(2)],
            sidebar_active: if self.sidebar_tab == 0 { self.page as usize } else { self.sidebar_rows[self.sidebar_tab.min(2)] },
            markup: ease(self.markup_progress()),
            ctx: self.ctx(),
            sheet,
        })
    }
    pub(super) fn send(&mut self, job: Job) -> bool {
        self.workers.send(job)
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
    state.pending = true;
    state.render_failed = false;
    state.status = "Opening...".into();
    let (width, height) = document::image_box(state, document);
    let request = Request { generation: state.generation, path, page: state.page, delta, width, height, sessions: state.sessions.clone() };
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
    let Some((path, dirty)) =
        with_state(|s| s.tabs.get(index).cloned().map(|p| (p.clone(), s.sessions.get(&p).is_some_and(|e| e.dirty)))).flatten()
    else {
        return;
    };
    if dirty
        && !sheet::confirm(
            hwnd,
            "Close this tab?",
            &format!("Your edits to {} are not saved. The original file stays unchanged.", file_name(&path)),
            "Close tab",
        )
    {
        return;
    }
    let next = with_state(|s| {
        s.tabs.retain(|p| *p != path);
        s.sessions.remove(&path);
        s.views.remove(&path);
        s.password_attempts.remove(&path);
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
                    state.status = match result {
                        Ok(()) => {
                            if let Some(current) = state.sessions.get_mut(&path) {
                                if whole_document && current.image == edits.image && current.pdf == edits.pdf {
                                    current.dirty = false;
                                }
                            }
                            "Saved a new copy. The original file is unchanged.".into()
                        }
                        Err(error) => error,
                    };
                    invalidate(hwnd);
                    continue;
                }
                Event::Text(generation, path, result) => {
                    if generation != state.generation || state.path.as_ref() != Some(&path) || state.pending {
                        continue;
                    }
                    state.status = match result {
                        Ok(text) if text.is_empty() => "No text found.".into(),
                        Ok(text) => match super::files::clipboard(hwnd, &text) {
                            Ok(()) => "Text copied. Check recognition and reading order before pasting.".into(),
                            Err(error) => error,
                        },
                        Err(error) => error,
                    };
                    invalidate(hwnd);
                    continue;
                }
                Event::Found(generation, result) => {
                    if generation == state.generation {
                        match result {
                            Ok(Some(page)) if state.pdf.is_some() => document::go_to_page(state, page, true),
                            Ok(Some(page)) => {
                                state.page = page;
                                schedule(hwnd, state, 0);
                            }
                            Ok(None) => state.status = "No matching text found.".into(),
                            Err(error) => state.status = error,
                        };
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
                    match result {
                        Ok(detail) => {
                            state.status = detail;
                            output_to_open = Some(output);
                        }
                        Err(error) => state.status = error,
                    }
                    invalidate(hwnd);
                    continue;
                }
                Event::Finished(result) => {
                    state.exporting = false;
                    state.cancel = None;
                    state.status = match result {
                        Ok(detail) => detail,
                        Err(error) => error,
                    };
                    invalidate(hwnd);
                    continue;
                }
                Event::Progress(text) => {
                    state.status = text;
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
        if state.tooltip.is_none() && state.hover_since.is_some_and(|t| t.elapsed() >= TOOLTIP_DELAY) {
            state.hover_since = None;
            state.tooltip = state.hover;
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
    if start_services {
        super::a11y::start(hwnd);
        super::window::watch_text_scale(hwnd);
    }
    if let Some((generation, fields)) = fields_to_show {
        super::actions::fill_field(hwnd, generation, fields);
    }
    if let Some(result) = info_to_show {
        super::actions::pdf_info(hwnd, result);
    }
    super::bench::tick(hwnd);
    if let Some(output) = output_to_open {
        open(hwnd, output);
    }
    if let Some((generation, path, page)) = password_to_show {
        password_prompt(hwnd, generation, path, page);
    }
    if advance {
        navigate(hwnd, 1);
    }
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
    let fresh = state.pdf.as_ref().is_some_and(|v| v.doc == item.key.doc && *v.edits == *item.edits);
    if !fresh {
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
        let request = Request { generation: state.generation, path, page, delta: 0, width: 1, height: 1, sessions: state.sessions.clone() };
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
