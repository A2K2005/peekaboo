//! Window-thread state, tabs, render scheduling, and worker results.
use super::{
    commands::{self, Command, Ctx},
    document::Pinch,
    render::{fonts, measure, Renderer},
    sheet::{self, Sheet},
    theme::Theme,
    widgets::{self, Layout, Scope, SheetView, WidgetId},
    worker::{is_pdf, Edits, Event, Job, Request, Workers},
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
    pub(super) zoom: f32,
    /// Zoom of the request in flight, and of the frame on screen. Pinch
    /// zoom scales the old frame by `zoom / frame_zoom` until the new one lands.
    pub(super) requested_zoom: f32,
    pub(super) frame_zoom: f32,
    pub(super) pan: (f32, f32),
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
    pub(super) views: HashMap<PathBuf, (u32, f32, (f32, f32))>,
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
    pub(super) markup_open: bool,
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
    pub(super) fn ctx(&self) -> Ctx {
        let pdf = self.is_pdf();
        Ctx {
            has_frame: self.frame.is_some(),
            pending: self.pending,
            failed: self.render_failed,
            pdf,
            saving: self.exporting,
            can_previous: !pdf || self.page > 0,
            can_next: !pdf || self.frame.as_ref().is_some_and(|f| self.page + 1 < f.page_count),
            tabs: self.tabs.len(),
            sidebar_open: self.sidebar_open,
            markup_open: self.markup_open,
            crop: self.crop,
            tool: self.tool(),
        }
    }
    /// "Page 3 of 20" or "1920 × 1080 pixels".
    pub(super) fn subtitle(&self) -> String {
        match &self.frame {
            Some(frame) if self.is_pdf() => format!("Page {} of {}", self.page + 1, frame.page_count),
            Some(frame) => format!("{} × {} pixels", frame.source_width, frame.source_height),
            None => String::new(),
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
            markup_open: self.markup_open,
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
    state.requested_zoom = state.zoom;
    let request = Request {
        generation: state.generation,
        path,
        page: state.page,
        delta,
        width: ((document.width() * state.zoom) as u32).clamp(1, 4096),
        height: ((document.height() * state.zoom) as u32).clamp(1, 4096),
        sessions: state.sessions.clone(),
    };
    if !state.send(Job::Render(request)) {
        state.pending = false;
        state.status = "The rendering worker stopped. Close Preview and reopen the file.".into();
    }
    invalidate(hwnd);
}

pub(super) unsafe fn open(hwnd: HWND, path: PathBuf) {
    with_state(|state| {
        if let Some((path, page)) = &state.displayed {
            if !state.pending && !state.render_failed {
                state.views.insert(path.clone(), (*page, state.zoom, state.pan));
            }
        }
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        let view = state.views.get(&path).copied().unwrap_or((0, 1.0, (0.0, 0.0)));
        state.path = Some(path);
        state.page = view.0;
        state.due = None;
        state.zoom = view.1;
        state.pan = view.2;
        state.crop = false;
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
            let count = state.frame.as_ref().map_or(0, |f| f.page_count);
            let page = state.page as i64 + delta as i64;
            if page < 0 || page >= count as i64 {
                return;
            }
            state.page = page as u32;
            schedule(hwnd, state, 0);
        } else {
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
    with_state(|state| {
        while let Ok(event) = state.workers.receiver.try_recv() {
            let completed = match event {
                Event::Render(completed) => completed,
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
                    let previous = state.displayed.as_ref().map(|(p, _)| p.clone());
                    let previous_dirty =
                        previous.as_ref().and_then(|p| state.sessions.get(p)).is_some_and(|edits| edits.dirty);
                    if completed.navigation {
                        if let Some((path, page)) = &state.displayed {
                            if path != &completed.path {
                                state.views.insert(path.clone(), (*page, state.zoom, state.pan));
                            }
                        }
                    }
                    state.password_attempts.remove(&completed.path);
                    state.displayed = Some((completed.path.clone(), completed.page));
                    let path = completed.path;
                    // Next or previous image replaces the tab, unless that
                    // tab holds unsaved edits.
                    if completed.navigation && !previous_dirty && !state.tabs.contains(&path) {
                        if let Some(index) = previous.and_then(|p| state.tabs.iter().position(|t| *t == p)) {
                            state.tabs[index] = path.clone();
                        }
                    }
                    if !state.tabs.contains(&path) {
                        state.tabs.push(path.clone());
                    }
                    title = Some(format!("{} - Preview for Windows", file_name(&path)));
                    state.path = Some(path);
                    state.page = completed.page;
                    state.render_failed = false;
                    state.frame = Some(frame);
                    state.frame_zoom = state.requested_zoom;
                    state.status = state.subtitle();
                    if let Some(renderer) = state.renderer.as_mut() {
                        renderer.bitmap = None;
                    }
                }
                Err(error) => {
                    state.render_failed = error != "No more images in this direction.";
                    state.status = format!("{error} Previous view retained.");
                    state.slideshow = None;
                    if error == crate::pdf::PASSWORD_REQUIRED {
                        password_to_show = Some((completed.generation, completed.path, completed.page));
                        state.due = None;
                    }
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
                }
            }
            invalidate(hwnd);
        }
        if state.painted && state.due.is_some_and(|due| Instant::now() >= due) {
            state.due = None;
            schedule(hwnd, state, 0);
        }
        if !state.pending && state.slideshow.is_some_and(|due| Instant::now() >= due) {
            if state.is_pdf() && state.frame.as_ref().is_some_and(|f| state.page + 1 >= f.page_count) {
                state.slideshow = None;
                state.status = "End of slideshow.".into();
            } else {
                state.slideshow = Some(Instant::now() + Duration::from_secs(3));
                advance = true;
            }
        }
        if state.tooltip.is_none() && state.hover_since.is_some_and(|t| t.elapsed() >= TOOLTIP_DELAY) {
            state.hover_since = None;
            state.tooltip = state.hover;
            invalidate(hwnd);
        }
        // Accessibility (UI Automation load) and UISettings (12 to 23 ms to
        // create on this PC) wait until the first content is on screen, so
        // they never delay it.
        let first_content = state.marked || state.frame.is_some() || state.render_failed || state.path.is_none();
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
        state.requested_zoom = state.zoom;
        let document = state.layout().document;
        let request = Request {
            generation: state.generation,
            path,
            page,
            delta: 0,
            width: (document.width().max(1.0) as u32).min(4096),
            height: (document.height().max(1.0) as u32).min(4096),
            sessions: state.sessions.clone(),
        };
        if !state.send(Job::Password(request, password)) {
            state.pending = false;
            state.status = "The PDF worker stopped.".into();
        }
    });
    invalidate(hwnd);
}
