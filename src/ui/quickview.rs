//! Quick view: the peek state of the main window. A popup sized to the
//! content, a hover strip, arrow keys through the selection or the folder,
//! an index sheet, and Open, which turns the same window into the editor
//! with the page and zoom kept. Docs: docs/quicklook-spec.md sections 1 and 5.
use super::{
    a11y, actions,
    app::{add_tabs, file_name, invalidate, open, with_state, State, EMPTY_STATUS},
    commands::{self, glyph, Chord, Command},
    document,
    paint::{icon_button, text_button, Look},
    render::{measure, Align},
    theme::{Mode, Rgba, Theme},
    view::{self, ViewMode, Zoom},
    widgets::{self, plain_widget, Layout, Rect, Region, Role, WidgetId},
    window::{is_full_screen, toggle_full_screen, PLACEMENT},
    worker::{is_pdf, WM_APP_WAKE},
};
use crate::integration::Action;
use crate::model::Frame;
use accesskit::{Node, NodeId, Role as NodeRole};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicIsize, Ordering},
        mpsc, Mutex,
    },
    time::{Duration, Instant},
};
use windows::{
    core::{w, HSTRING},
    Win32::{
        Foundation::{COLORREF, HWND, LPARAM, RECT, SIZE, WPARAM},
        Graphics::{
            Direct2D::ID2D1Bitmap,
            Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DEFAULT, DWMWCP_ROUND},
            Gdi::{
                DeleteObject, GetDC, GetDIBits, GetMonitorInfoW, GetObjectW, MonitorFromWindow, ReleaseDC, BITMAP, BITMAPINFO,
                BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, MONITORINFO, MONITOR_DEFAULTTONEAREST,
            },
        },
        System::{
            Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
            Threading::{AttachThreadInput, GetCurrentThreadId},
        },
        UI::{
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::{
                GetKeyState, SendInput, SetFocus, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
                VIRTUAL_KEY, VK_CONTROL, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_MENU, VK_NEXT, VK_PRIOR,
                VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
            },
            Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_RESIZETOFIT},
            WindowsAndMessaging::*,
        },
    },
};

/// Posted when a peek or a handed-off file is waiting for the window thread.
pub(super) const WM_APP_INCOMING: u32 = WM_APP + 0x72;

/// Sizes in epx, from the spec's visual table.
const STRIP: f32 = 40.0;
const STRIP_PAD: f32 = 12.0;
const STRIP_BUTTON: f32 = 32.0;
const STRIP_GAP: f32 = 4.0;
const OPEN_BUTTON: f32 = 64.0;
const MIN_SIZE: (f32, f32) = (360.0, 240.0);
/// The editor's window is at least this wide, so its toolbar fits.
const EDITOR_WIDTH: f32 = 640.0;
const GRID_THUMB: f32 = 160.0;
const GRID_CELL: f32 = 184.0;

/// Fluent durations: 120 ms in, 83 ms out, 167 ms strip exit.
/// https://learn.microsoft.com/windows/apps/design/motion/timing-and-easing
const OPEN_FADE: Duration = Duration::from_millis(120);
const CLOSE_FADE: Duration = Duration::from_millis(83);
const STRIP_IN: Duration = Duration::from_millis(120);
const STRIP_OUT: Duration = Duration::from_millis(167);
const STRIP_HOLD: Duration = Duration::from_millis(1500);

/// Commands a key may run in Quick view. Everything else that edits waits
/// for the editor.
const KEYS: &[Command] = &[
    Command::ZoomIn,
    Command::ZoomOut,
    Command::ActualSize,
    Command::Fit,
    Command::FitWidth,
    Command::CopyText,
    Command::Print,
    Command::Share,
    Command::FileInfo,
    Command::Rotate,
    Command::ToggleMarkup,
    Command::FullScreen,
];

const INDEX_NODE: NodeId = NodeId(83_010);

static WINDOW: AtomicIsize = AtomicIsize::new(0);
static RESIDENT: AtomicBool = AtomicBool::new(false);
static PENDING: Mutex<Option<(PathBuf, Vec<PathBuf>)>> = Mutex::new(None);

thread_local! {
    static HANDOFFS: RefCell<Vec<crate::integration::Command>> = const { RefCell::new(Vec::new()) };
}

pub(super) struct Quick {
    files: Vec<PathBuf>,
    index: usize,
    /// The file the window was last sized for.
    sized: Option<PathBuf>,
    shown: bool,
    /// Window fade: start, and true while opening.
    fade: Option<(Instant, bool)>,
    /// Hover strip: when it appeared and when the pointer last moved.
    reveal: Option<Instant>,
    moved: Instant,
    strip_drawn: f32,
    grid: Option<Grid>,
    thumbs: Vec<Option<Frame>>,
    bitmaps: Vec<Option<ID2D1Bitmap>>,
    thumbs_rx: Option<mpsc::Receiver<(usize, Option<Frame>)>>,
}

struct Grid {
    focus: usize,
    scroll: f32,
}

impl Quick {
    fn new(files: Vec<PathBuf>, index: usize) -> Self {
        let now = Instant::now();
        let count = files.len();
        Self {
            files,
            index,
            sized: None,
            shown: false,
            fade: None,
            reveal: Some(now),
            moved: now,
            strip_drawn: 0.0,
            grid: None,
            thumbs: vec![None; count],
            bitmaps: vec![None; count],
            thumbs_rx: None,
        }
    }
    fn closing(&self) -> bool {
        matches!(self.fade, Some((_, false)))
    }
    /// Bitmaps belong to one render target; a new target uploads again.
    pub(super) fn forget_bitmaps(&mut self) {
        self.bitmaps.iter_mut().for_each(|b| *b = None);
    }
}

/// Queues a peek from any thread and wakes the window thread.
pub(super) fn post(path: PathBuf, siblings: Vec<PathBuf>) {
    *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some((path, siblings));
    wake();
}

fn wake() {
    let window = WINDOW.load(Ordering::SeqCst);
    if window != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(window as *mut _)), WM_APP_INCOMING, WPARAM(0), LPARAM(0));
        }
    }
}

/// The window exists: peeks queued before it now run.
pub(super) fn attach(hwnd: HWND, resident: bool) {
    RESIDENT.store(resident, Ordering::SeqCst);
    WINDOW.store(hwnd.0 as isize, Ordering::SeqCst);
    wake();
}

/// A command from another process (`integration::hand_off`). It runs from
/// the message queue, so the sender is not held while dialogs open.
pub(super) fn received(hwnd: HWND, command: crate::integration::Command) {
    if command.action == Action::Peek {
        if let Some((first, rest)) = command.paths.split_first() {
            post(first.clone(), rest.to_vec());
        }
        return;
    }
    HANDOFFS.with(|q| q.borrow_mut().push(command));
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_APP_INCOMING, WPARAM(0), LPARAM(0));
    }
}

/// Runs queued peeks and handoffs. A sheet holds them until it closes; tick
/// calls this again.
pub(super) unsafe fn incoming(hwnd: HWND) {
    if with_state(|s| s.sheet.is_some()).unwrap_or(true) {
        return;
    }
    let peek = PENDING.lock().unwrap_or_else(|e| e.into_inner()).take();
    let handoffs = HANDOFFS.with(|q| std::mem::take(&mut *q.borrow_mut()));
    if peek.is_some() || !handoffs.is_empty() {
        // The hidden resident window runs no timer; tick shows the window.
        SetTimer(Some(hwnd), 1, 10, None);
    }
    if let Some((path, siblings)) = peek {
        start(hwnd, path, siblings);
    }
    for command in handoffs {
        open_command(hwnd, command);
    }
}

/// Supported files next to `path`, by name.
fn folder_files(path: &Path) -> Vec<PathBuf> {
    let Some(Ok(entries)) = path.parent().map(std::fs::read_dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .filter(|p| super::organize::can_insert(p))
        .collect();
    files.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    files
}

unsafe fn start(hwnd: HWND, path: PathBuf, siblings: Vec<PathBuf>) {
    let mut files = if siblings.is_empty() { folder_files(&path) } else { siblings };
    let index = files.iter().position(|f| *f == path).unwrap_or_else(|| {
        files.insert(0, path.clone());
        0
    });
    let visible = IsWindowVisible(hwnd).as_bool();
    let mode = with_state(|s| (s.quick.as_ref().map(Quick::closing), s.tabs.is_empty()));
    match mode {
        // The editor holds documents: the file opens there as a tab.
        Some((None, false)) if visible => {
            with_state(|s| add_tabs(s, std::slice::from_ref(&path)));
            open(hwnd, path);
            activate_window(hwnd);
            return;
        }
        Some((Some(false), _)) if visible => {
            with_state(|s| {
                let q = s.quick.as_mut()?;
                *q = Quick { sized: q.sized.take(), shown: true, fade: q.fade.take(), ..Quick::new(files, index) };
                Some(())
            });
            show_file(hwnd);
            activate_window(hwnd);
            return;
        }
        _ => {}
    }
    if visible {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
    if is_full_screen() {
        toggle_full_screen(hwnd);
    }
    set_layered(hwnd, false);
    with_state(|s| {
        reset(s);
        s.quick = Some(Quick::new(files, index));
        s.focus = Some(WidgetId::Document);
        s.focus_visible = false;
    });
    set_style(hwnd, true);
    // Hidden at the largest Quick view size on the user's monitor, so the
    // first decode is big enough; tick shrinks it to the content.
    let work = work_area(MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTONEAREST));
    let size = ((work.right - work.left) as f32 * 0.8, (work.bottom - work.top) as f32 * 0.85);
    place(hwnd, size, work, Some(((work.left + work.right) / 2, (work.top + work.bottom) / 2)));
    show_file(hwnd);
}

/// Opens the current file of the list at the Quick view zoom.
unsafe fn show_file(hwnd: HWND) {
    let path = with_state(|s| {
        let q = s.quick.as_ref()?;
        let path = q.files.get(q.index)?.clone();
        // A file seen before opens fresh, not at its last zoom.
        s.views.remove(&std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone()));
        Some(path)
    })
    .flatten();
    let Some(path) = path else {
        return;
    };
    let pdf = is_pdf(&path);
    open(hwnd, path);
    with_state(|s| {
        if pdf {
            // The whole first page shows, as in Quick Look.
            s.zoom = Zoom::Fit;
            s.view_mode = ViewMode::Continuous;
        }
    });
    invalidate(hwnd);
}

/// Back to an empty, hidden window with no documents.
fn reset(s: &mut State) {
    s.generation = s.generation.wrapping_add(1);
    s.quick = None;
    s.path = None;
    s.frame = None;
    s.pdf = None;
    s.displayed = None;
    s.pending = false;
    s.render_failed = false;
    s.tabs.clear();
    s.sessions.clear();
    s.saves.clear();
    s.views.clear();
    s.password_attempts.clear();
    s.markup = None;
    s.markup_open = false;
    s.markup_since = None;
    s.sidebar_open = false;
    s.crop = false;
    s.signature = None;
    s.slideshow = None;
    s.selection = None;
    s.text_selection = None;
    s.focus = None;
    s.document_ring = false;
    s.zoom = Zoom::Fit;
    s.pan = (0.0, 0.0);
    s.forms = Default::default();
    // close() pauses autosave while it asks; the hidden window starts fresh.
    s.pause_save_dispatch(false);
    s.status = EMPTY_STATUS.into();
    s.messages = Default::default();
    s.recent = super::empty::load();
    if let Some(renderer) = s.renderer.as_mut() {
        renderer.bitmap = None;
    }
}

fn work_area(monitor: windows::Win32::Graphics::Gdi::HMONITOR) -> RECT {
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let _ = GetMonitorInfoW(monitor, &mut info);
    }
    info.rcWork
}

/// Sizes the window so its client area is `client`, centered on `center`
/// (the window's own center when None), inside `work`.
unsafe fn place(hwnd: HWND, client: (f32, f32), work: RECT, center: Option<(i32, i32)>) {
    let (mut window, mut inner) = (RECT::default(), RECT::default());
    let _ = GetWindowRect(hwnd, &mut window);
    let _ = GetClientRect(hwnd, &mut inner);
    let extra = ((window.right - window.left) - inner.right, (window.bottom - window.top) - inner.bottom);
    let (work_w, work_h) = (work.right - work.left, work.bottom - work.top);
    let w = (client.0 as i32 + extra.0).min(work_w);
    let h = (client.1 as i32 + extra.1).min(work_h);
    let (cx, cy) = center.unwrap_or(((window.left + window.right) / 2, (window.top + window.bottom) / 2));
    let x = (cx - w / 2).clamp(work.left, work.right - w);
    let y = (cy - h / 2).clamp(work.top, work.bottom - h);
    let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
}

/// Quick view is a resizable popup; the editor is a normal window. Full
/// screen keeps its own style, so the change goes to the saved one.
unsafe fn set_style(hwnd: HWND, quick: bool) {
    let frame = (WS_OVERLAPPEDWINDOW.0 | WS_POPUP.0) as isize;
    let wanted = if quick { (WS_POPUP | WS_THICKFRAME).0 } else { WS_OVERLAPPEDWINDOW.0 } as isize;
    let saved = PLACEMENT.with(|f| {
        let mut f = f.borrow_mut();
        let (_, style) = f.as_mut()?;
        *style = (*style & !frame) | wanted;
        Some(())
    });
    // Windows 11 rounds popups only on request; Windows 10 ignores it.
    let corner = if quick { DWMWCP_ROUND } else { DWMWCP_DEFAULT };
    let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &corner as *const _ as _, std::mem::size_of_val(&corner) as u32);
    if saved.is_some() {
        return;
    }
    let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
    if style & frame == wanted {
        return;
    }
    SetWindowLongPtrW(hwnd, GWL_STYLE, (style & !frame) | wanted);
    let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
}

unsafe fn set_layered(hwnd: HWND, on: bool) {
    let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
    let layered = WS_EX_LAYERED.0 as isize;
    if on == (ex & layered != 0) {
        return;
    }
    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, if on { ex | layered } else { ex & !layered });
}

unsafe fn set_alpha(hwnd: HWND, alpha: f32) {
    let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), (alpha.clamp(0.0, 1.0) * 255.0).round() as u8, LWA_ALPHA);
}

/// Marks the Alt tap `activate_window` injects, so the window does not
/// treat it as a keytip press.
pub(super) const ACTIVATION_TAP: usize = 0x5046_5741;

unsafe fn activate_window(hwnd: HWND) {
    let _ = ShowWindow(hwnd, if IsIconic(hwnd).as_bool() { SW_RESTORE } else { SW_SHOW });
    for retry in [false, true] {
        if GetForegroundWindow() == hwnd {
            break;
        }
        bring_forward(hwnd, retry);
    }
    if crate::resident::logging() {
        let shown = IsWindowVisible(hwnd).as_bool();
        crate::resident::log(&format!("show: shown {shown}, foreground after show {}", GetForegroundWindow() == hwnd));
    }
}

/// A background process started by a keyboard hook may not take the
/// foreground: Windows refuses SetForegroundWindow and the window opens
/// behind Explorer. Sharing the foreground thread's input state is not
/// always enough, so the retry also taps Alt, which lifts the foreground
/// lock, and raises the window through the topmost band.
/// https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-setforegroundwindow
unsafe fn bring_forward(hwnd: HWND, alt: bool) {
    let current = GetCurrentThreadId();
    let foreground = GetWindowThreadProcessId(GetForegroundWindow(), None);
    let attached = foreground != current && AttachThreadInput(current, foreground, true).as_bool();
    if alt {
        tap_alt(KEYBD_EVENT_FLAGS(0));
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW;
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, flags);
        let _ = SetWindowPos(hwnd, Some(HWND_NOTOPMOST), 0, 0, 0, 0, flags);
    }
    let _ = SetForegroundWindow(hwnd);
    let _ = BringWindowToTop(hwnd);
    let _ = SetFocus(Some(hwnd));
    if alt {
        tap_alt(KEYEVENTF_KEYUP);
    }
    if attached {
        let _ = AttachThreadInput(current, foreground, false);
    }
}

unsafe fn tap_alt(flags: KEYBD_EVENT_FLAGS) {
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VK_MENU, dwFlags: flags, dwExtraInfo: ACTIVATION_TAP, ..Default::default() } },
    };
    SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
}

/// The client size for the content: images at 100% when they fit, else in
/// 80% of the work area; PDFs one whole page, at most 80% of its width
/// and 85% of its height.
fn content_size(s: &State, work: (f32, f32)) -> (f32, f32) {
    let (ww, wh) = work;
    let size = match (&s.pdf, &s.frame) {
        (Some(v), _) => {
            // The scale Zoom::Fit picks in this window: the whole page fits.
            let gap = document::gap(s);
            let widest = v.sizes.iter().map(|p| p[0]).fold(1.0, f32::max);
            let tallest = v.sizes.iter().map(|p| p[1]).fold(1.0, f32::max);
            let scale = view::actual(s.scale).min((0.8 * ww - 2.0 * gap) / widest).min((0.85 * wh - 2.0 * gap) / tallest).max(0.0);
            (widest * scale + 2.0 * gap, tallest * scale + 2.0 * gap)
        }
        (None, Some(f)) => {
            let (w, h) = (f.source_width.max(1) as f32, f.source_height.max(1) as f32);
            let k = (0.8 * ww / w).min(0.8 * wh / h).min(1.0);
            (w * k, h * k)
        }
        _ => (0.0, 0.0),
    };
    (size.0.max(MIN_SIZE.0 * s.scale).round(), size.1.max(MIN_SIZE.1 * s.scale).round())
}

fn strip_alpha(q: &Quick, animations: bool, now: Instant) -> f32 {
    let Some(reveal) = q.reveal else {
        return 0.0;
    };
    let idle = now.saturating_duration_since(q.moved);
    match (animations, idle < STRIP_HOLD) {
        (false, held) => f32::from(u8::from(held)),
        (true, true) => (now.saturating_duration_since(reveal).as_secs_f32() / STRIP_IN.as_secs_f32()).min(1.0),
        (true, false) => (1.0 - (idle - STRIP_HOLD).as_secs_f32() / STRIP_OUT.as_secs_f32()).max(0.0),
    }
}

/// The strip takes input from the moment it starts to appear until it is gone.
fn strip_shown(q: &Quick, animations: bool, now: Instant) -> bool {
    strip_alpha(q, animations, now) > 0.0 || (q.reveal.is_some() && now.saturating_duration_since(q.moved) < STRIP_HOLD)
}

/// Pointer movement shows the hover strip.
pub(super) unsafe fn reveal(hwnd: HWND) {
    let appeared = with_state(|s| {
        let animations = s.animations;
        let q = s.quick.as_mut()?;
        let now = Instant::now();
        let hidden = strip_alpha(q, animations, now) <= 0.0;
        if hidden {
            q.reveal = Some(now);
        }
        q.moved = now;
        Some(hidden)
    })
    .flatten();
    if appeared == Some(true) {
        invalidate(hwnd);
    }
}

enum Fade {
    Step(f32),
    Opened,
    Closed,
}

/// Places the window for new content, runs the fades, and drains thumbnails.
pub(super) unsafe fn tick(hwnd: HWND) {
    incoming(hwnd);
    let Some((placement, fade, repaint)) = with_state(|s| {
        let now = Instant::now();
        let animations = s.animations;
        let ready = !s.pending && (s.frame.is_some() || s.pdf.is_some());
        let shown_path = s.displayed.as_ref().map(|(p, _)| p.clone());
        let failed = s.render_failed;
        let strip_focus = strip_has(s, s.hover) || strip_has(s, s.caption_hover) || (s.focus_visible && strip_has(s, s.focus));
        let q = s.quick.as_mut()?;
        let mut repaint = false;
        if let Some(rx) = &q.thumbs_rx {
            while let Ok((index, frame)) = rx.try_recv() {
                if let Some(slot) = q.thumbs.get_mut(index) {
                    *slot = frame;
                    repaint = true;
                }
            }
        }
        let mut placement = None;
        if !q.closing() && ((ready && shown_path.is_some() && shown_path != q.sized) || (failed && !q.shown)) {
            q.sized = shown_path;
            placement = Some(!std::mem::replace(&mut q.shown, true));
        }
        if strip_focus {
            q.moved = now;
            q.reveal.get_or_insert(now);
        }
        let alpha = strip_alpha(q, animations, now);
        if alpha <= 0.0 && now.saturating_duration_since(q.moved) >= STRIP_HOLD {
            q.reveal = None;
        }
        if (alpha - q.strip_drawn).abs() > 0.01 || (alpha == 0.0) != (q.strip_drawn == 0.0) {
            q.strip_drawn = alpha;
            repaint = true;
        }
        let fade = q.fade.map(|(since, opening)| {
            let length = if opening { OPEN_FADE } else { CLOSE_FADE };
            let t = (since.elapsed().as_secs_f32() / length.as_secs_f32()).min(1.0);
            match (t >= 1.0, opening) {
                (true, true) => Fade::Opened,
                (true, false) => Fade::Closed,
                // cubic-bezier(0,0,0,1) in, cubic-bezier(1,0,1,1) out.
                (false, true) => Fade::Step(super::app::ease(t)),
                (false, false) => Fade::Step(1.0 - t * t * t),
            }
        });
        if matches!(fade, Some(Fade::Opened | Fade::Closed)) {
            q.fade = None;
        }
        Some((placement, fade, repaint))
    })
    .flatten() else {
        return;
    };
    if let Some(first) = placement {
        if !is_full_screen() {
            let work = work_area(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST));
            let size = with_state(|s| content_size(s, ((work.right - work.left) as f32, (work.bottom - work.top) as f32)));
            if let Some(size) = size {
                place(hwnd, size, work, None);
            }
        }
        if first {
            begin_show(hwnd);
        }
    }
    match fade {
        Some(Fade::Step(alpha)) => set_alpha(hwnd, alpha),
        Some(Fade::Opened) => set_layered(hwnd, false),
        Some(Fade::Closed) => finish(hwnd),
        None => {}
    }
    if repaint {
        invalidate(hwnd);
    }
}

fn strip_has(s: &State, id: Option<WidgetId>) -> bool {
    s.quick.is_some()
        && id.is_some_and(|id| matches!(id, WidgetId::Close | WidgetId::Command(_)))
        && s.layout().widgets.iter().any(|w| Some(w.id) == id && w.region == Region::TitleBar)
}

unsafe fn begin_show(hwnd: HWND) {
    if with_state(|s| s.animations).unwrap_or(false) {
        set_layered(hwnd, true);
        set_alpha(hwnd, 0.0);
        with_state(|s| {
            if let Some(q) = s.quick.as_mut() {
                q.fade = Some((Instant::now(), true));
            }
        });
    }
    activate_window(hwnd);
}

/// The window closes: Quick view fades out first. The resident process
/// keeps its window hidden for the next peek; any other process exits.
pub(super) unsafe fn dismiss(hwnd: HWND) {
    let fade = with_state(|s| {
        let animations = s.animations;
        let q = s.quick.as_mut().filter(|q| animations && q.shown)?;
        q.fade = Some((Instant::now(), false));
        Some(())
    })
    .flatten();
    if fade.is_some() && IsWindowVisible(hwnd).as_bool() {
        set_layered(hwnd, true);
        set_alpha(hwnd, 1.0);
        return;
    }
    finish(hwnd);
}

unsafe fn finish(hwnd: HWND) {
    if !RESIDENT.load(Ordering::SeqCst) {
        let _ = DestroyWindow(hwnd);
        return;
    }
    if is_full_screen() {
        toggle_full_screen(hwnd);
    }
    let _ = ShowWindow(hwnd, SW_HIDE);
    let _ = KillTimer(Some(hwnd), 1);
    set_layered(hwnd, false);
    with_state(reset);
    let _ = SetWindowTextW(hwnd, w!("Preview for Windows"));
}

/// Quick view commands, and Markup, which leaves Quick view. Returns true
/// when handled.
pub(super) unsafe fn command(hwnd: HWND, command: Command) -> bool {
    let quick = with_state(|s| s.quick.is_some()).unwrap_or(false);
    match command {
        Command::OpenInEditor => open_editor(hwnd, false),
        Command::IndexSheet => toggle_grid(hwnd),
        Command::ToggleMarkup if quick => open_editor(hwnd, true),
        _ => return false,
    }
    true
}

/// Turns Quick view into the editor in place: same document, page, and zoom.
unsafe fn open_editor(hwnd: HWND, markup: bool) {
    let Some(title) = with_state(|s| {
        let has_content = s.frame.is_some() || s.pdf.is_some();
        // Measured while the Quick view layout still applies.
        let ratio = document::ratio(s);
        s.quick.take()?;
        if has_content && s.zoom != Zoom::Ratio(ratio) {
            s.zoom = Zoom::Ratio(ratio);
        }
        if let Some((path, _)) = &s.displayed {
            s.tabs = vec![path.clone()];
        }
        s.focus = Some(WidgetId::Document);
        if markup {
            s.set_markup(true);
        }
        Some(s.path.as_deref().map(|p| format!("{} - Preview for Windows", file_name(p))))
    })
    .flatten() else {
        return;
    };
    set_layered(hwnd, false);
    set_style(hwnd, false);
    if !is_full_screen() {
        let (mut client, mut window) = (RECT::default(), RECT::default());
        let _ = GetClientRect(hwnd, &mut client);
        let _ = GetWindowRect(hwnd, &mut window);
        let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
        let chrome = with_state(|s| s.layout().document.y0).unwrap_or(48.0 * scale);
        let size = ((client.right as f32).max(EDITOR_WIDTH * scale), client.bottom as f32 + chrome);
        let work = work_area(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST));
        place(hwnd, size, work, None);
    }
    if let Some(title) = title {
        let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
        let _ = SetWindowTextW(hwnd, windows::core::PCWSTR(title.as_ptr()));
    }
    activate_window(hwnd);
    invalidate(hwnd);
}

/// Opens files handed off by another launch in the editor.
unsafe fn open_command(hwnd: HWND, command: crate::integration::Command) {
    let quick = with_state(|s| s.quick.is_some()).unwrap_or(false);
    if quick {
        open_editor(hwnd, false);
    } else if !IsWindowVisible(hwnd).as_bool() {
        set_style(hwnd, false);
        let work = work_area(MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTONEAREST));
        let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
        let center = ((work.left + work.right) / 2, (work.top + work.bottom) / 2);
        place(hwnd, (1100.0 * scale, 800.0 * scale), work, Some(center));
    }
    let paths: Vec<PathBuf> = command.paths.into_iter().map(|p| std::fs::canonicalize(&p).unwrap_or(p)).collect();
    match command.action {
        Action::Convert | Action::Resize => {
            with_state(|s| s.tools.verb = Some((command.action == Action::Resize, paths)));
        }
        _ => {
            with_state(|s| add_tabs(s, &paths));
            if let Some(first) = paths.into_iter().next() {
                open(hwnd, first);
            }
        }
    }
    activate_window(hwnd);
    invalidate(hwnd);
}

/// Keys in Quick view. Returns true when handled; the window's own key
/// handling runs otherwise.
pub(super) unsafe fn key(hwnd: HWND, vk: u16, repeat: bool) -> bool {
    let Some((grid, closing, strip_focus)) = with_state(|s| {
        let q = s.quick.as_ref().filter(|_| s.sheet.is_none())?;
        Some((q.grid.as_ref().map(|g| g.focus), q.closing(), s.focus_visible && strip_has(s, s.focus)))
    })
    .flatten() else {
        return false;
    };
    if closing {
        return true;
    }
    let down = |key: VIRTUAL_KEY| GetKeyState(key.0 as i32) < 0;
    let (ctrl, shift, alt) = (down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU));
    // Other Alt keys fall through to the editor; its Alt+Up and Alt+Down page shortcuts must not.
    if alt {
        return matches!(VIRTUAL_KEY(vk), VK_UP | VK_DOWN);
    }
    match VIRTUAL_KEY(vk) {
        VK_SPACE if !ctrl => {
            // Held Space repeats into the new window; only a fresh press closes it.
            if !repeat {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        VK_ESCAPE if is_full_screen() => toggle_full_screen(hwnd),
        VK_ESCAPE if grid.is_some() => toggle_grid(hwnd),
        VK_ESCAPE => {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        key @ (VK_LEFT | VK_UP | VK_RIGHT | VK_DOWN) if !ctrl => match grid {
            Some(_) => move_grid(hwnd, key),
            None => step(hwnd, if matches!(key, VK_RIGHT | VK_DOWN) { 1 } else { -1 }),
        },
        VK_RETURN if ctrl => toggle_grid(hwnd),
        VK_RETURN if strip_focus => return false,
        VK_RETURN => match grid {
            Some(focus) => pick(hwnd, focus),
            None => open_editor(hwnd, false),
        },
        VK_TAB => {
            reveal(hwnd);
            return false;
        }
        VK_PRIOR | VK_NEXT | VK_HOME | VK_END if !ctrl && grid.is_none() => {
            with_state(|s| document::key(s, vk, shift));
        }
        _ => match commands::lookup(Chord { key: vk, ctrl, shift, alt }) {
            Some(command) if KEYS.contains(&command) => actions::execute(hwnd, command, true),
            Some(_) => {}
            None => return false,
        },
    }
    invalidate(hwnd);
    true
}

/// Next or previous file in the list.
unsafe fn step(hwnd: HWND, delta: i32) {
    let moved = with_state(|s| {
        let q = s.quick.as_mut()?;
        let next = usize::try_from(q.index as i64 + delta as i64).ok().filter(|n| *n < q.files.len())?;
        q.index = next;
        Some(())
    })
    .flatten();
    if moved.is_some() {
        show_file(hwnd);
    }
}

/// Opens or closes the index sheet. It needs 2 or more files.
unsafe fn toggle_grid(hwnd: HWND) {
    let load = with_state(|s| {
        let q = s.quick.as_mut()?;
        if q.grid.take().is_some() {
            s.focus = Some(WidgetId::Document);
            return None;
        }
        if q.files.len() < 2 {
            return None;
        }
        q.grid = Some(Grid { focus: q.index, scroll: 0.0 });
        s.focus = Some(WidgetId::IndexItem(q.index));
        if q.thumbs_rx.is_some() {
            return None;
        }
        let (tx, rx) = mpsc::channel();
        q.thumbs_rx = Some(rx);
        Some((q.files.clone(), tx, (GRID_THUMB * s.scale) as i32))
    })
    .flatten();
    if let Some((files, tx, side)) = load {
        load_thumbnails(hwnd.0 as isize, files, side, tx);
    }
    with_state(follow);
    invalidate(hwnd);
}

/// Shows the index sheet file `index` in Quick view.
pub(super) unsafe fn pick(hwnd: HWND, index: usize) {
    let picked = with_state(|s| {
        let q = s.quick.as_mut().filter(|q| index < q.files.len())?;
        q.grid = None;
        q.index = index;
        s.focus = Some(WidgetId::Document);
        Some(())
    })
    .flatten();
    if picked.is_some() {
        show_file(hwnd);
    }
}

unsafe fn move_grid(hwnd: HWND, key: VIRTUAL_KEY) {
    with_state(|s| {
        let columns = cells(s).columns as i64;
        let q = s.quick.as_mut()?;
        let count = q.files.len() as i64;
        let grid = q.grid.as_mut()?;
        let delta = match key {
            VK_LEFT => -1,
            VK_RIGHT => 1,
            VK_UP => -columns,
            _ => columns,
        };
        grid.focus = (grid.focus as i64 + delta).clamp(0, count - 1) as usize;
        s.focus = Some(WidgetId::IndexItem(grid.focus));
        s.focus_visible = true;
        follow(s);
        Some(())
    });
    invalidate(hwnd);
}

/// The wheel scrolls the index sheet. Returns true when it did.
pub(super) fn wheel(s: &mut State, delta: f32) -> bool {
    let c = cells(s);
    let Some(grid) = s.quick.as_mut().and_then(|q| q.grid.as_mut()) else {
        return false;
    };
    let max = (c.total - c.area.height()).max(0.0);
    grid.scroll = (grid.scroll - delta / 120.0 * c.cell.1 / 2.0).clamp(0.0, max);
    true
}

/// Scrolls the index sheet so its focused cell is in view.
fn follow(s: &mut State) {
    let c = cells(s);
    let Some(grid) = s.quick.as_mut().and_then(|q| q.grid.as_mut()) else {
        return;
    };
    let top = (grid.focus / c.columns) as f32 * c.cell.1;
    let view = c.area.height();
    if top < grid.scroll {
        grid.scroll = top;
    } else if top + c.cell.1 > grid.scroll + view {
        grid.scroll = (top + c.cell.1 - view).max(0.0);
    }
}

struct Cells {
    area: Rect,
    cell: (f32, f32),
    columns: usize,
    total: f32,
}

fn cells(s: &State) -> Cells {
    let (scale, count) = (s.scale, s.quick.as_ref().map_or(0, |q| q.files.len()));
    let area = Rect { x0: 16.0 * scale, y0: STRIP * scale, x1: (s.size.0 - 16.0 * scale).max(16.0 * scale), y1: s.size.1.max(STRIP * scale) };
    let cell = (GRID_CELL * scale, (GRID_THUMB + 16.0 + 20.0 * s.text_scale + 12.0) * scale);
    let columns = ((area.width() / cell.0).floor() as usize).max(1);
    Cells { area, cell, columns, total: count.div_ceil(columns) as f32 * cell.1 }
}

/// Index sheet cells in view, as (file index, rectangle).
fn visible_cells(s: &State) -> Vec<(usize, Rect)> {
    let c = cells(s);
    let Some((count, scroll)) = s.quick.as_ref().and_then(|q| Some((q.files.len(), q.grid.as_ref()?.scroll))) else {
        return Vec::new();
    };
    let left = c.area.x0 + (c.area.width() - c.columns as f32 * c.cell.0).max(0.0) / 2.0;
    (0..count)
        .map(|i| {
            let (row, column) = (i / c.columns, i % c.columns);
            (i, Rect::new(left + column as f32 * c.cell.0, c.area.y0 + row as f32 * c.cell.1 - scroll, c.cell.0, c.cell.1))
        })
        .filter(|(_, r)| r.y1 > c.area.y0 && r.y0 < c.area.y1)
        .collect()
}

/// Replaces the editor layout: the document fills the window, the strip
/// floats over its top, and the index sheet replaces the document.
pub(super) fn layout(s: &State, q: &Quick, layout: &mut Layout) {
    let (w, h, scale) = (s.size.0.max(1.0), s.size.1.max(1.0), s.scale);
    let full = Rect::new(0.0, 0.0, w, h);
    let modal = layout.sheet.is_some();
    let grid = q.grid.is_some();
    layout.widgets.retain(|x| x.region == Region::Sheet || x.id == WidgetId::Document);
    if let Some(document) = layout.widgets.iter_mut().find(|x| x.id == WidgetId::Document) {
        document.rect = if grid { Rect::default() } else { full };
        document.focusable &= !grid;
    }
    let strip = Rect::new(0.0, 0.0, w, (STRIP * scale).min(h));
    // The strip drags the window, except in full screen.
    layout.title_bar = if is_full_screen() { Rect::default() } else { strip };
    layout.title_text = Rect::default();
    layout.tab_strip = None;
    layout.markup_bar = None;
    layout.sidebar = None;
    layout.sidebar_edge = None;
    layout.sidebar_panel = Rect::default();
    layout.contact_sheet = false;
    layout.document = full;
    layout.info_area = full;
    layout.empty = None;
    layout.toolbar_overflow.clear();
    layout.markup_overflow.clear();
    let mut strip_widgets = Vec::new();
    if strip_shown(q, s.animations, Instant::now()) {
        let button = STRIP_BUTTON * scale;
        let y = (strip.height() - button) / 2.0;
        let ctx = s.ctx();
        let mut x = w - STRIP_PAD * scale;
        let mut next = |width: f32| {
            x -= width;
            let rect = Rect::new(x, y, width, button);
            x -= STRIP_GAP * scale;
            rect
        };
        let mut close = plain_widget(WidgetId::Close, Role::Button, Region::TitleBar, next(button), "Close".into());
        close.glyph = Some(glyph::CANCEL);
        close.tooltip = "Close (Esc)".into();
        strip_widgets.push(close);
        let mut commands = vec![Command::FullScreen, Command::Share];
        if !s.is_pdf() {
            commands.push(Command::Rotate);
        }
        commands.extend([Command::ToggleMarkup, Command::OpenInEditor]);
        for command in commands {
            let width = if command == Command::OpenInEditor { OPEN_BUTTON * scale } else { button };
            strip_widgets.push(widgets::command_widget(command, Region::TitleBar, next(width), &ctx));
        }
        let right = x;
        let mut left = STRIP_PAD * scale;
        if q.files.len() > 1 {
            let rect = Rect::new(left, y, button, button);
            strip_widgets.push(widgets::command_widget(Command::IndexSheet, Region::TitleBar, rect, &ctx));
            left = rect.x1;
        }
        strip_widgets.reverse();
        // The room between the buttons; paint centers the name in the
        // window when it fits there, so a small window still shows it.
        let gap = 8.0 * scale;
        layout.title_text = Rect { x0: left + gap, y0: 0.0, x1: (right - gap).max(left + gap), y1: strip.y1 };
    }
    for widget in &mut strip_widgets {
        widget.focusable &= !modal;
    }
    layout.widgets.splice(0..0, strip_widgets);
    if grid {
        let focus = q.grid.as_ref().map(|g| g.focus);
        let area = cells(s).area;
        for (index, rect) in visible_cells(s) {
            let rect = Rect { y0: rect.y0.max(area.y0), y1: rect.y1.min(area.y1), ..rect };
            let mut item = plain_widget(WidgetId::IndexItem(index), Role::ListItem, Region::Document, rect, file_name(&q.files[index]));
            item.checked = Some(index == q.index);
            item.focusable = !modal && focus == Some(index);
            layout.widgets.push(item);
        }
    }
}

/// The strip's colors at `alpha` of their opacity.
fn faded(t: Theme, alpha: f32) -> Theme {
    let f = |c: Rgba| c.alpha(c.3 * alpha);
    Theme {
        text: f(t.text),
        text_secondary: f(t.text_secondary),
        text_disabled: f(t.text_disabled),
        hover: f(t.hover),
        hover_text: f(t.hover_text),
        pressed: f(t.pressed),
        accent: f(t.accent),
        on_accent: f(t.on_accent),
        field: f(t.field),
        control_border: f(t.control_border),
        ..t
    }
}

/// Draws the document or the index sheet, then the strip. Returns true
/// when the document shows complete content.
pub(super) fn paint(l: &Look, bitmap: Option<&ID2D1Bitmap>, s: &mut State, layout: &Layout) -> bool {
    let grid = s.quick.as_ref().is_some_and(|q| q.grid.is_some());
    let drew = if grid {
        paint_grid(l, s, layout);
        false
    } else {
        document::paint(l.p, bitmap, s, layout.document)
    };
    let Some(q) = s.quick.as_ref() else {
        return drew;
    };
    let alpha = strip_alpha(q, s.animations, Instant::now());
    if alpha <= 0.0 {
        return drew;
    }
    let strip = Rect::new(0.0, 0.0, layout.document.width(), STRIP * l.s);
    let back = if l.t.mode == Mode::Contrast { l.t.bar } else { l.t.surface.alpha(0.85 * alpha) };
    l.p.fill(strip, back);
    l.p.fill(Rect { y0: strip.y1 - l.s, ..strip }, l.t.divider.alpha(l.t.divider.3 * alpha));
    let look = Look { p: l.p, t: faded(l.t, alpha), f: l.f, s: l.s };
    let name = q.files.get(q.index).map(|p| file_name(p)).unwrap_or_default();
    let room = layout.title_text;
    let width = (measure(&name, &l.f.strong, 100_000.0).0.ceil() + l.s).min(room.width());
    let x0 = (strip.width() / 2.0 - width / 2.0).min(room.x1 - width).max(room.x0);
    l.p.text(&name, Rect { x0, x1: x0 + width, ..room }, &l.f.strong, look.t.text, Align::Center);
    for w in layout.widgets.iter().filter(|w| w.region == Region::TitleBar) {
        if w.id == WidgetId::Command(Command::OpenInEditor) {
            text_button(&look, s, w);
        } else {
            icon_button(&look, s, w);
        }
    }
    drew
}

fn paint_grid(l: &Look, s: &mut State, layout: &Layout) {
    let (scale, ts) = (l.s, s.text_scale);
    let area = cells(s).area;
    let cells = visible_cells(s);
    let hover = s.hover;
    l.p.fill(layout.document, l.t.canvas);
    let Some(q) = s.quick.as_mut() else {
        return;
    };
    let side = GRID_THUMB * scale;
    l.p.push_clip(area);
    for (index, r) in cells {
        if hover == Some(WidgetId::IndexItem(index)) {
            l.p.fill_round(r.inset(4.0 * scale), 8.0 * scale, l.t.hover);
        }
        let frame = Rect::new(r.x0 + (r.width() - side) / 2.0, r.y0 + 8.0 * scale, side, side);
        if q.bitmaps[index].is_none() {
            if let Some(thumb) = &q.thumbs[index] {
                q.bitmaps[index] = l.p.upload(thumb).ok();
            }
        }
        match (&q.bitmaps[index], &q.thumbs[index]) {
            (Some(bitmap), Some(thumb)) => {
                let k = (side / thumb.width.max(1) as f32).min(side / thumb.height.max(1) as f32);
                let (w, h) = (thumb.width as f32 * k, thumb.height as f32 * k);
                let image = Rect::new(frame.x0 + (side - w) / 2.0, frame.y0 + (side - h) / 2.0, w, h);
                l.p.draw_bitmap(bitmap, image);
                l.p.stroke_round(image, 0.0, l.t.image_outline, scale);
            }
            _ => l.p.glyph(glyph::DOCUMENT, frame, &l.f.icon, l.t.text_secondary),
        }
        if index == q.index {
            l.p.stroke_round(frame.inset(-4.0 * scale), 6.0 * scale, l.t.accent, 2.0 * scale);
        }
        let label = Rect { x0: r.x0 + 4.0 * scale, y0: frame.y1 + 8.0 * scale, x1: r.x1 - 4.0 * scale, y1: frame.y1 + (8.0 + 20.0 * ts) * scale };
        l.p.text(&file_name(&q.files[index]), label, &l.f.caption, l.t.text, Align::Center);
    }
    l.p.pop_clip();
}

/// The index sheet as a list for UI Automation.
pub(super) fn a11y(layout: &Layout, nodes: &mut Vec<(NodeId, Node)>, children: &mut Vec<NodeId>) {
    let items: Vec<NodeId> =
        layout.widgets.iter().filter(|w| matches!(w.id, WidgetId::IndexItem(_))).map(|w| a11y::node_id(w.id)).collect();
    if items.is_empty() {
        return;
    }
    let mut node = Node::new(NodeRole::List);
    node.set_label("Index sheet");
    node.set_bounds(a11y::bounds(layout.document));
    node.set_children(items);
    nodes.push((INDEX_NODE, node));
    children.push(INDEX_NODE);
}

/// Shell thumbnails for the index sheet, read on their own thread; each
/// result wakes the window thread.
fn load_thumbnails(window: isize, files: Vec<PathBuf>, side: i32, tx: mpsc::Sender<(usize, Option<Frame>)>) {
    std::thread::spawn(move || unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        for (index, path) in files.iter().enumerate() {
            if tx.send((index, shell_thumbnail(path, side))).is_err() {
                break;
            }
            let _ = PostMessageW(Some(HWND(window as *mut _)), WM_APP_WAKE, WPARAM(0), LPARAM(0));
        }
        if com {
            CoUninitialize();
        }
    });
}

/// The thumbnail Explorer shows for `path`, or its icon, as premultiplied BGRA.
/// https://learn.microsoft.com/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishellitemimagefactory-getimage
unsafe fn shell_thumbnail(path: &Path, side: i32) -> Option<Frame> {
    // The shell does not parse verbatim paths, which canonicalize returns.
    let text = path.to_string_lossy();
    let plain = match text.strip_prefix(r"\\?\UNC\") {
        Some(rest) => format!(r"\\{rest}"),
        None => text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned(),
    };
    let factory: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(plain), None).ok()?;
    let bitmap = factory.GetImage(SIZE { cx: side, cy: side }, SIIGBF_RESIZETOFIT).ok()?;
    let frame = bitmap_frame(bitmap);
    let _ = DeleteObject(bitmap.into());
    frame
}

unsafe fn bitmap_frame(bitmap: HBITMAP) -> Option<Frame> {
    let mut info = BITMAP::default();
    if GetObjectW(bitmap.into(), std::mem::size_of::<BITMAP>() as i32, Some(&mut info as *mut BITMAP as *mut _)) == 0 {
        return None;
    }
    let (width, height) = (info.bmWidth.max(1) as u32, info.bmHeight.unsigned_abs().max(1));
    let mut header = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            // Negative height: rows top to bottom.
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let dc = GetDC(None);
    let rows = GetDIBits(dc, bitmap, 0, height, Some(pixels.as_mut_ptr().cast()), &mut header, DIB_RGB_COLORS);
    ReleaseDC(None, dc);
    if rows == 0 {
        return None;
    }
    // Opaque thumbnails often come with a zero alpha channel.
    if pixels.chunks_exact(4).all(|p| p[3] == 0) {
        pixels.chunks_exact_mut(4).for_each(|p| p[3] = 255);
    }
    Some(Frame { width, height, pixels, page_count: 1, source_width: width, source_height: height })
}
