//! Quick view: the peek state of the main window. A popup sized to the
//! content under a header that stays above it, a page rail for multi-page
//! PDFs, arrow keys through the selection or the folder, an index sheet,
//! and Open, which hands the file to its default app or turns the same
//! window into the editor with the page and zoom kept. Files the editor does
//! not open show as text, through their system preview handler, or as an
//! info card. Docs: docs/quicklook-spec.md sections 1, 5, and 7.
use super::{
    a11y, actions,
    app::{add_tabs, display_path, file_name, invalidate, open, with_state, State, EMPTY_STATUS},
    commands::{self, glyph, Chord, Command, MenuItem, Pick},
    document,
    handler::{self, Host},
    infocard::{self, Card},
    paint::{icon_button, text_button, Look},
    render::{fonts, measure, Align},
    sheet,
    textview::{self, TextView},
    theme::{Mode, Rgba, Theme},
    view::{self, ViewMode, Zoom},
    widgets::{self, plain_widget, Layout, Rect, Region, Role, Widget, WidgetId},
    window::{is_full_screen, toggle_full_screen, PLACEMENT},
    worker::{is_pdf, Key, Work, WM_APP_WAKE},
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
    core::{w, BOOL, GUID, HSTRING},
    Win32::{
        Foundation::{COLORREF, HWND, LPARAM, POINT, RECT, SIZE, WPARAM},
        Graphics::{
            Direct2D::ID2D1Bitmap,
            Dwm::{
                DwmSetWindowAttribute, DWMWA_TRANSITIONS_FORCEDISABLED, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DEFAULT,
                DWMWCP_ROUND,
            },
            Gdi::{
                ClientToScreen, DeleteObject, GetDC, GetDIBits, GetMonitorInfoW, GetObjectW, InvalidateRect, MonitorFromRect,
                MonitorFromWindow, ReleaseDC, UpdateWindow, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP,
                MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL,
            },
        },
        System::{
            Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED},
            Threading::{AttachThreadInput, GetCurrentThreadId},
        },
        UI::{
            Accessibility::{CUIAutomation, IUIAutomation, UIA_ListItemControlTypeId},
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

/// Sizes in epx. The header matches the reference frames (spec section
/// 7.1): close and full screen, the name, then More, Share, and Open.
const HEADER: f32 = 36.0;
const HEADER_PAD: f32 = 8.0;
const HEADER_BUTTON: f32 = 28.0;
const HEADER_GAP: f32 = 4.0;
const PILL_PAD: f32 = 12.0;
/// The space between the content and the window edges, and between pages.
pub(super) const INSET: f32 = 6.0;
const RAIL: f32 = 90.0;
const RAIL_PAD: f32 = 8.0;
const MIN_SIZE: (f32, f32) = (360.0, 240.0);
/// The editor's window is at least this wide, so its toolbar fits.
const EDITOR_WIDTH: f32 = 640.0;
const GRID_THUMB: f32 = 160.0;
const GRID_CELL: f32 = 184.0;

/// Open and close stay under 200 ms, near Fluent's 167 ms; the header
/// fades in and out in full screen only.
/// https://learn.microsoft.com/windows/apps/design/motion/timing-and-easing
const OPEN_TIME: Duration = Duration::from_millis(150);
const CLOSE_TIME: Duration = Duration::from_millis(120);
/// Without the item's rectangle, the window grows from this share of its size.
const START_SCALE: f32 = 0.96;
const START_ALPHA: f32 = 0.3;
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
/// Set while a motion moves the window. WM_SIZE then leaves the layout and
/// the render target at the final size, so the window shows the content
/// scaled to its current size.
static MOVING: AtomicBool = AtomicBool::new(false);
/// The default app's name by lowercase extension, read off the window
/// thread once per Quick view session. None: this app, or no app.
static APPS: Mutex<Vec<(String, Option<String>)>> = Mutex::new(Vec::new());

thread_local! {
    static HANDOFFS: RefCell<Vec<crate::integration::Command>> = const { RefCell::new(Vec::new()) };
}

pub(super) struct Quick {
    files: Vec<PathBuf>,
    index: usize,
    /// The file the window opened on. Close shrinks to its item only.
    opened: usize,
    /// The file the window was last sized for.
    sized: Option<PathBuf>,
    shown: bool,
    motion: Option<Motion>,
    /// The item's rectangle in Explorer, in screen pixels, once UI
    /// Automation answers.
    origin: Option<RECT>,
    origin_rx: Option<mpsc::Receiver<RECT>>,
    /// Full-screen header: when it appeared and when the pointer last moved.
    reveal: Option<Instant>,
    moved: Instant,
    strip_drawn: f32,
    grid: Option<Grid>,
    thumbs: Vec<Option<Frame>>,
    bitmaps: Vec<Option<ID2D1Bitmap>>,
    thumbs_rx: Option<mpsc::Receiver<(usize, Option<Frame>)>>,
    /// The current file when the editor does not open its type.
    other: Option<Other>,
}

/// The window moving between two window rectangles, in screen pixels.
#[derive(Clone, Copy)]
struct Motion {
    since: Instant,
    opening: bool,
    from: RECT,
    to: RECT,
}

struct Grid {
    focus: usize,
    scroll: f32,
}

impl Quick {
    /// Starts reading the rectangle of the item focused in `foreground`.
    fn new(files: Vec<PathBuf>, index: usize, foreground: HWND) -> Self {
        let now = Instant::now();
        let count = files.len();
        Self {
            files,
            index,
            opened: index,
            sized: None,
            shown: false,
            motion: None,
            origin: None,
            origin_rx: Some(find_origin(foreground)),
            reveal: Some(now),
            moved: now,
            strip_drawn: 0.0,
            grid: None,
            thumbs: vec![None; count],
            bitmaps: vec![None; count],
            thumbs_rx: None,
            other: None,
        }
    }
    fn closing(&self) -> bool {
        self.motion.is_some_and(|m| !m.opening)
    }
    /// Bitmaps belong to one render target; a new target uploads again.
    pub(super) fn forget_bitmaps(&mut self) {
        self.bitmaps.iter_mut().for_each(|b| *b = None);
        if let Some(other) = &mut self.other {
            other.icon = None;
        }
    }
}

/// A file or folder the editor does not open: text, a system preview
/// handler, or the info card. It loads off the window thread.
pub(super) struct Other {
    path: PathBuf,
    rx: mpsc::Receiver<Loaded>,
    text: Option<TextView>,
    host: Option<Host>,
    card: Option<Card>,
    icon: Option<ID2D1Bitmap>,
}

enum Loaded {
    Text(String),
    Handler(GUID, Card),
    /// The card, then again with its icon.
    Card(Card),
}

impl Other {
    fn new(hwnd: HWND, path: PathBuf, icon_side: i32) -> Self {
        let (tx, rx) = mpsc::channel();
        let (window, item) = (hwnd.0 as isize, path.clone());
        std::thread::spawn(move || unsafe {
            let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
            let wake = || {
                let _ = PostMessageW(Some(HWND(window as *mut _)), WM_APP_WAKE, WPARAM(0), LPARAM(0));
            };
            let first = classify(&item);
            let card = match &first {
                Loaded::Handler(_, card) | Loaded::Card(card) => Some(card.clone()),
                Loaded::Text(_) => None,
            };
            if tx.send(first).is_ok() {
                wake();
                if let Some(mut card) = card {
                    card.icon = shell_thumbnail(&item, icon_side);
                    if tx.send(Loaded::Card(card)).is_ok() {
                        wake();
                    }
                }
            }
            if com {
                CoUninitialize();
            }
        });
        Self { path, rx, text: None, host: None, card: None, icon: None }
    }

    fn ready(&self) -> bool {
        self.text.is_some() || self.card.is_some()
    }

    /// Takes what the loader sent. Returns true when the view changed.
    fn receive(&mut self, hwnd: HWND, doc: Rect) -> bool {
        let mut changed = false;
        while let Ok(loaded) = self.rx.try_recv() {
            changed = true;
            match loaded {
                Loaded::Text(text) => self.text = Some(TextView::new(text)),
                Loaded::Handler(clsid, card) => {
                    self.host = Some(Host::start(hwnd, clsid, PathBuf::from(display_path(&self.path)), doc));
                    self.card = Some(card);
                }
                Loaded::Card(card) => {
                    self.card = Some(card);
                    self.icon = None;
                }
            }
        }
        changed
    }

    /// The content size in pixels for a work area of `work`.
    fn size(&self, scale: f32, work: (f32, f32)) -> (f32, f32) {
        match (&self.text, &self.host) {
            (Some(_), _) => (0.45 * work.0, 0.75 * work.1),
            (None, Some(_)) => (0.6 * work.0, 0.8 * work.1),
            (None, None) => (infocard::SIZE.0 * scale, infocard::SIZE.1 * scale),
        }
    }

    fn paint(&mut self, l: &Look, r: Rect, text_scale: f32) {
        if let Some(text) = &mut self.text {
            return text.paint(l, r, text_scale);
        }
        // The handler's own window covers `r`.
        if self.host.as_ref().is_some_and(Host::ready) {
            return;
        }
        let Some(card) = &self.card else {
            return;
        };
        if self.icon.is_none() {
            self.icon = card.icon.as_ref().and_then(|frame| l.p.upload(frame).ok());
        }
        card.paint(l, self.icon.as_ref(), r);
    }
}

/// Folders and unreadable files get the card. Text types show as text even
/// when a handler is registered; other types prefer their handler.
fn classify(path: &Path) -> Loaded {
    let Some(head) = (!path.is_dir()).then(|| textview::head(path)).flatten() else {
        return Loaded::Card(Card::read(path));
    };
    let known = textview::known(path) && textview::is_text(&head, true);
    if let Some(clsid) = (!known).then(|| handler::find(path)).flatten() {
        return Loaded::Handler(clsid, Card::read(path));
    }
    if known || textview::is_text(&head, false) {
        if let Ok(text) = textview::read(path) {
            return Loaded::Text(text);
        }
    }
    Loaded::Card(Card::read(path))
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

/// True while the open or close motion moves the window.
pub(super) fn moving() -> bool {
    MOVING.load(Ordering::SeqCst)
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

/// The items next to `path`: folders, then files, each by name.
fn folder_files(path: &Path) -> Vec<PathBuf> {
    let Some(Ok(entries)) = path.parent().map(std::fs::read_dir) else {
        return Vec::new();
    };
    let mut items: Vec<(bool, String, PathBuf)> = entries
        .filter_map(|e| e.ok())
        .map(|e| (!e.file_type().is_ok_and(|t| t.is_dir()), e.file_name().to_string_lossy().to_lowercase(), e.path()))
        .collect();
    items.sort();
    items.into_iter().map(|(.., path)| path).collect()
}

unsafe fn start(hwnd: HWND, path: PathBuf, siblings: Vec<PathBuf>) {
    let mut files = if siblings.is_empty() { folder_files(&path) } else { siblings };
    let index = files.iter().position(|f| *f == path).unwrap_or_else(|| {
        files.insert(0, path.clone());
        0
    });
    let visible = IsWindowVisible(hwnd).as_bool();
    let foreground = GetForegroundWindow();
    let mode = with_state(|s| (s.quick.as_ref().map(Quick::closing), s.tabs.is_empty()));
    match mode {
        // The editor holds documents: the file opens there as a tab, or
        // in its own app when the editor cannot open it.
        Some((None, false)) if visible && !super::organize::can_insert(&path) => {
            if let Err(error) = crate::integration::open_with_default(&path) {
                sheet::alert(hwnd, "Open", &error);
            }
            return;
        }
        Some((None, false)) if visible => {
            with_state(|s| add_tabs(s, std::slice::from_ref(&path)));
            open(hwnd, path);
            activate_window(hwnd);
            return;
        }
        Some((Some(false), _)) if visible => {
            with_state(|s| {
                let q = s.quick.as_mut()?;
                *q = Quick { sized: q.sized.take(), shown: true, motion: q.motion.take(), ..Quick::new(files, index, foreground) };
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
    MOVING.store(false, Ordering::SeqCst);
    set_layered(hwnd, false);
    APPS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    with_state(|s| {
        reset(s);
        s.quick = Some(Quick::new(files, index, foreground));
        s.focus = Some(WidgetId::Document);
        s.focus_visible = false;
    });
    set_style(hwnd, true);
    // Hidden at the largest Quick view size on the user's monitor, so the
    // first decode is big enough; tick shrinks it to the content.
    let work = work_area(MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST));
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
        s.sidebar_scroll[0] = 0.0;
        Some(path)
    })
    .flatten();
    let Some(path) = path else {
        return;
    };
    look_up_app(hwnd, &path);
    if !super::organize::can_insert(&path) {
        let title: Vec<u16> = format!("{} - Preview for Windows", file_name(&path)).encode_utf16().chain(Some(0)).collect();
        let _ = SetWindowTextW(hwnd, windows::core::PCWSTR(title.as_ptr()));
        with_state(|s| {
            forget_document(s);
            let side = (infocard::ICON * s.scale) as i32;
            if let Some(q) = s.quick.as_mut() {
                q.other = Some(Other::new(hwnd, path, side));
            }
        });
        invalidate(hwnd);
        return;
    }
    with_state(|s| s.quick.as_mut().map(|q| q.other = None));
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

/// No document on screen; late worker results for the last one are dropped.
fn forget_document(s: &mut State) {
    s.generation = s.generation.wrapping_add(1);
    s.path = None;
    s.frame = None;
    s.pdf = None;
    s.displayed = None;
    s.pending = false;
    s.render_failed = false;
    s.selection = None;
    s.text_selection = None;
    if let Some(renderer) = s.renderer.as_mut() {
        renderer.bitmap = None;
    }
}

/// Back to an empty, hidden window with no documents.
fn reset(s: &mut State) {
    forget_document(s);
    s.quick = None;
    s.tabs.clear();
    s.sessions.clear();
    s.saves.clear();
    s.views.clear();
    s.password_attempts.clear();
    s.markup = None;
    s.markup_open = false;
    s.markup_since = None;
    s.sidebar_open = false;
    // The page rail is the thumbnail list.
    s.sidebar_tab = 0;
    s.sidebar_scroll = [0.0; 4];
    s.crop = false;
    s.signature = None;
    s.slideshow = None;
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
    // Quick view runs its own open and close motion; a system one would add to it.
    let transitions_off = BOOL::from(quick);
    let _ = DwmSetWindowAttribute(
        hwnd,
        DWMWA_TRANSITIONS_FORCEDISABLED,
        &transitions_off as *const _ as _,
        std::mem::size_of_val(&transitions_off) as u32,
    );
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

/// Header height in pixels. It grows with the text size setting.
fn header_height(s: &State) -> f32 {
    (HEADER.max(20.0 * s.text_scale + 12.0) * s.scale).round()
}

/// The page rail's width in pixels: multi-page PDFs only.
fn rail_width(s: &State) -> f32 {
    if s.pdf.as_ref().is_some_and(|v| v.sizes.len() > 1) {
        (RAIL * s.scale).round()
    } else {
        0.0
    }
}

/// The page rail, left of the content and under the header.
fn rail(s: &State) -> Option<Rect> {
    let width = rail_width(s);
    let grid = s.quick.as_ref().is_some_and(|q| q.grid.is_some());
    (width > 0.0 && !grid).then(|| {
        let top = if is_full_screen() { 0.0 } else { header_height(s) };
        Rect { x0: 0.0, y0: top, x1: width.min(s.size.0 / 2.0), y1: s.size.1.max(top) }
    })
}

/// Every page's thumbnail rectangle in the rail, in order, at the rail's
/// scroll offset. Pages fit the rail's width and at most 1.4 times as tall.
fn rail_rows(s: &State, rail: Rect) -> impl Iterator<Item = (usize, Rect)> + '_ {
    let sizes = s.pdf.as_ref().map_or(&[][..], |v| &v.sizes[..]);
    let pad = RAIL_PAD * s.scale;
    let side = (rail.width() - 2.0 * pad).max(1.0);
    let mut top = rail.y0 + pad - s.sidebar_scroll[0];
    sizes.iter().enumerate().map(move |(page, size)| {
        let k = (side / size[0].max(1.0)).min(1.4 * side / size[1].max(1.0));
        let (w, h) = ((size[0] * k).round().max(1.0), (size[1] * k).round().max(1.0));
        let r = Rect::new((rail.x0 + (rail.width() - w) / 2.0).round(), top.round(), w, h);
        top += h + pad;
        (page, r)
    })
}

/// Scrolls the rail just enough to show the current page.
pub(super) fn follow_rail(s: &mut State) {
    let Some(rail) = rail(s) else {
        return;
    };
    let Some((_, row)) = rail_rows(s, rail).nth(s.page as usize) else {
        return;
    };
    let pad = RAIL_PAD * s.scale;
    if row.y0 - pad < rail.y0 {
        s.sidebar_scroll[0] -= rail.y0 - (row.y0 - pad);
    } else if row.y1 + pad > rail.y1 {
        s.sidebar_scroll[0] += row.y1 + pad - rail.y1;
    }
}

/// The client size for the content under the header: images at 100% when
/// they fit, else in 80% of the work area; PDFs one whole page, at most 80%
/// of its width and 85% of its height. The scale matches what Zoom::Fit
/// picks at the hidden first size, so the first render is never redone.
fn content_size(s: &State, work: (f32, f32)) -> (f32, f32) {
    let (ww, wh) = work;
    let (header, rail, gap) = (header_height(s), rail_width(s), document::gap(s));
    let room = |height: f32| (0.8 * ww - rail - 2.0 * gap, height * wh - header - 2.0 * gap);
    let other = s.quick.as_ref().and_then(|q| q.other.as_ref());
    let size = match (other, &s.pdf, &s.frame) {
        (Some(other), ..) => other.size(s.scale, work),
        (None, Some(v), _) => {
            let (room_w, room_h) = room(0.85);
            let widest = v.sizes.iter().map(|p| p[0]).fold(1.0, f32::max);
            let tallest = v.sizes.iter().map(|p| p[1]).fold(1.0, f32::max);
            let scale = view::actual(s.scale).min(room_w / widest).min(room_h / tallest).max(0.0);
            (widest * scale, tallest * scale)
        }
        (None, None, Some(f)) => {
            let (room_w, room_h) = room(0.8);
            let (w, h) = (f.source_width.max(1) as f32, f.source_height.max(1) as f32);
            let k = (room_w / w).min(room_h / h).clamp(0.0, 1.0);
            (w * k, h * k)
        }
        _ => (0.0, 0.0),
    };
    let width = size.0 + 2.0 * gap + rail;
    let height = size.1 + 2.0 * gap + header;
    (width.max(MIN_SIZE.0 * s.scale).round(), height.max(MIN_SIZE.1 * s.scale).round())
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

/// The full-screen header takes input from the moment it starts to appear until it is gone.
fn strip_shown(q: &Quick, animations: bool, now: Instant) -> bool {
    strip_alpha(q, animations, now) > 0.0 || (q.reveal.is_some() && now.saturating_duration_since(q.moved) < STRIP_HOLD)
}

/// The header stays in a window; in full screen it shows with the pointer.
fn header_alpha(q: &Quick, animations: bool, now: Instant) -> f32 {
    if is_full_screen() {
        strip_alpha(q, animations, now)
    } else {
        1.0
    }
}

/// Pointer movement shows the full-screen header.
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
        Some(hidden && is_full_screen())
    })
    .flatten();
    if appeared == Some(true) {
        invalidate(hwnd);
    }
}

/// Places the window for new content, runs the motion, and drains
/// thumbnails and the item's rectangle.
pub(super) unsafe fn tick(hwnd: HWND) {
    incoming(hwnd);
    let Some((placement, mut step, repaint)) = with_state(|s| {
        let now = Instant::now();
        let animations = s.animations;
        let ready = !s.pending && (s.frame.is_some() || s.pdf.is_some());
        let shown_path = s.displayed.as_ref().map(|(p, _)| p.clone());
        let failed = s.render_failed;
        let strip_focus = strip_has(s, s.hover) || strip_has(s, s.caption_hover) || (s.focus_visible && strip_has(s, s.focus));
        let doc = s.layout().document;
        let q = s.quick.as_mut()?;
        let mut repaint = false;
        let mut target = shown_path.filter(|_| ready);
        let visible = !moving() && q.grid.is_none() && !q.closing();
        if let Some(other) = q.other.as_mut() {
            repaint |= other.receive(hwnd, doc);
            target = other.ready().then(|| other.path.clone());
            if let Some(host) = other.host.as_mut() {
                host.place(doc, visible);
            }
        }
        if let Some(rx) = &q.thumbs_rx {
            while let Ok((index, frame)) = rx.try_recv() {
                if let Some(slot) = q.thumbs.get_mut(index) {
                    *slot = frame;
                    repaint = true;
                }
            }
        }
        if let Some(rect) = q.origin_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            q.origin = Some(rect);
        }
        let mut placement = None;
        if !q.closing() && ((target.is_some() && target != q.sized) || (failed && !q.shown)) {
            q.sized = target;
            placement = Some(!std::mem::replace(&mut q.shown, true));
        }
        if strip_focus {
            q.moved = now;
            q.reveal.get_or_insert(now);
        }
        let alpha = header_alpha(q, animations, now);
        if alpha <= 0.0 && now.saturating_duration_since(q.moved) >= STRIP_HOLD {
            q.reveal = None;
        }
        if (alpha - q.strip_drawn).abs() > 0.01 || (alpha == 0.0) != (q.strip_drawn == 0.0) {
            q.strip_drawn = alpha;
            repaint = true;
        }
        let step = q.motion.map(|m| {
            let length = if m.opening { OPEN_TIME } else { CLOSE_TIME };
            (m, (m.since.elapsed().as_secs_f32() / length.as_secs_f32()).min(1.0))
        });
        // New content needs the final size at once, not at the end of the motion.
        if step.is_some_and(|(_, t)| t >= 1.0) || placement.is_some() {
            q.motion = None;
        }
        Some((placement, step, repaint))
    })
    .flatten() else {
        return;
    };
    if placement.is_some() {
        if let Some((m, _)) = step.take() {
            end_motion(hwnd, m);
        }
    }
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
    match step {
        Some((m, t)) if t >= 1.0 => end_motion(hwnd, m),
        Some((m, t)) => {
            // cubic-bezier(0,0,0,1)-like ease out to open; ease in to close.
            let (k, alpha) = if m.opening {
                (super::app::ease(t), START_ALPHA + t / 0.4)
            } else {
                (t * t * t, (1.0 - t) / 0.4)
            };
            move_to(hwnd, lerp(m.from, m.to, k));
            set_alpha(hwnd, alpha);
            invalidate(hwnd);
            // Paint before DWM composes the new size, so no stale frame shows.
            let _ = UpdateWindow(hwnd);
        }
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

fn lerp(a: RECT, b: RECT, k: f32) -> RECT {
    let f = |x: i32, y: i32| x + ((y - x) as f32 * k).round() as i32;
    RECT { left: f(a.left, b.left), top: f(a.top, b.top), right: f(a.right, b.right), bottom: f(a.bottom, b.bottom) }
}

fn scaled(r: RECT, k: f32) -> RECT {
    let (w, h) = ((r.right - r.left) as f32 * k, (r.bottom - r.top) as f32 * k);
    let (x, y) = ((r.left + r.right) as f32 / 2.0 - w / 2.0, (r.top + r.bottom) as f32 / 2.0 - h / 2.0);
    RECT { left: x.round() as i32, top: y.round() as i32, right: (x + w).round() as i32, bottom: (y + h).round() as i32 }
}

unsafe fn move_to(hwnd: HWND, r: RECT) {
    let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
}

/// The window rectangle whose client area fits in `item` with the aspect
/// ratio of `content`, so the motion only scales. None when `item` is on
/// another monitor: a motion across monitors would change the DPI.
unsafe fn around(hwnd: HWND, item: RECT, content: (f32, f32)) -> Option<RECT> {
    if MonitorFromRect(&item, MONITOR_DEFAULTTONULL) != MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) {
        return None;
    }
    let (mut window, mut client, mut origin) = (RECT::default(), RECT::default(), POINT::default());
    let _ = GetWindowRect(hwnd, &mut window);
    let _ = GetClientRect(hwnd, &mut client);
    let _ = ClientToScreen(hwnd, &mut origin);
    let (width, height) = ((item.right - item.left) as f32, (item.bottom - item.top) as f32);
    let k = (width / content.0.max(1.0)).min(height / content.1.max(1.0));
    let (w, h) = (content.0 * k, content.1 * k);
    let (x, y) = (item.left as f32 + (width - w) / 2.0, item.top as f32 + (height - h) / 2.0);
    Some(RECT {
        left: (x - (origin.x - window.left) as f32).round() as i32,
        top: (y - (origin.y - window.top) as f32).round() as i32,
        right: (x + w + (window.right - origin.x - client.right) as f32).round() as i32,
        bottom: (y + h + (window.bottom - origin.y - client.bottom) as f32).round() as i32,
    })
}

unsafe fn start_motion(hwnd: HWND, opening: bool, from: RECT, to: RECT) {
    MOVING.store(true, Ordering::SeqCst);
    set_layered(hwnd, true);
    set_alpha(hwnd, if opening { START_ALPHA } else { 1.0 });
    move_to(hwnd, from);
    with_state(|s| {
        if let Some(q) = s.quick.as_mut() {
            q.motion = Some(Motion { since: Instant::now(), opening, from, to });
        }
    });
}

unsafe fn end_motion(hwnd: HWND, m: Motion) {
    if !m.opening {
        return finish(hwnd);
    }
    // Still MOVING here, so the window's layout keeps its size.
    move_to(hwnd, m.to);
    MOVING.store(false, Ordering::SeqCst);
    set_layered(hwnd, false);
    invalidate(hwnd);
}

/// Ends a running motion at its last frame.
unsafe fn settle(hwnd: HWND) {
    if let Some(m) = with_state(|s| s.quick.as_mut().and_then(|q| q.motion.take())).flatten() {
        end_motion(hwnd, m);
    }
}

/// The window grows from the item in Explorer, or from slightly smaller
/// than its size when the item's rectangle is not known yet.
unsafe fn begin_show(hwnd: HWND) {
    let origin = with_state(|s| s.animations.then(|| (s.quick.as_ref().and_then(|q| q.origin), s.size))).flatten();
    if let Some((origin, content)) = origin {
        let mut to = RECT::default();
        let _ = GetWindowRect(hwnd, &mut to);
        let from = origin.and_then(|item| around(hwnd, item, content)).unwrap_or_else(|| scaled(to, START_SCALE));
        start_motion(hwnd, true, from, to);
    }
    activate_window(hwnd);
}

/// The window closes: Quick view shrinks back to the item it opened from,
/// or fades. The resident process keeps its window hidden for the next
/// peek; any other process exits.
pub(super) unsafe fn dismiss(hwnd: HWND) {
    let target = with_state(|s| {
        let animations = s.animations;
        let q = s.quick.as_ref().filter(|q| animations && q.shown && !q.closing())?;
        Some((q.origin.filter(|_| q.index == q.opened), s.size))
    })
    .flatten();
    let Some((origin, content)) = target.filter(|_| IsWindowVisible(hwnd).as_bool()) else {
        return finish(hwnd);
    };
    let mut from = RECT::default();
    let _ = GetWindowRect(hwnd, &mut from);
    let to = match origin {
        _ if is_full_screen() => from,
        Some(item) => around(hwnd, item, content).unwrap_or_else(|| scaled(from, START_SCALE)),
        None => scaled(from, START_SCALE),
    };
    start_motion(hwnd, false, from, to);
}

unsafe fn finish(hwnd: HWND) {
    MOVING.store(false, Ordering::SeqCst);
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

/// The rectangle of the item focused in Explorer or on the desktop, in
/// screen pixels, read with UI Automation on its own thread so the first
/// frame never waits for it. File lists expose items as ListItem elements;
/// any other focus gives no rectangle.
fn find_origin(foreground: HWND) -> mpsc::Receiver<RECT> {
    let (tx, rx) = mpsc::channel();
    let foreground = foreground.0 as isize;
    std::thread::spawn(move || unsafe {
        let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        if let Some(rect) = focused_item(HWND(foreground as *mut _)) {
            let _ = tx.send(rect);
        }
        if com {
            CoUninitialize();
        }
    });
    rx
}

unsafe fn focused_item(foreground: HWND) -> Option<RECT> {
    let automation: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
    let element = automation.GetFocusedElement().ok()?;
    let mut process = 0u32;
    GetWindowThreadProcessId(foreground, Some(&mut process));
    if element.CurrentProcessId().ok()? as u32 != process
        || element.CurrentControlType().ok()? != UIA_ListItemControlTypeId
        || element.CurrentIsOffscreen().ok()?.as_bool()
    {
        return None;
    }
    let r = element.CurrentBoundingRectangle().ok()?;
    // A Details row spans every column; its icon sits at the left end.
    let side = (r.right - r.left).min(r.bottom - r.top);
    (side > 0).then_some(RECT { left: r.left, top: r.top, right: r.left + side, bottom: r.top + side })
}

fn extension(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// The app that opens `path` when it is not this one, once known.
fn default_app(path: &Path) -> Option<String> {
    let extension = extension(path);
    APPS.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(e, _)| *e == extension).and_then(|(_, app)| app.clone())
}

/// Reads the default app's name for `path`'s type on its own thread, then
/// repaints, so the header names it without delaying the first frame.
fn look_up_app(hwnd: HWND, path: &Path) {
    let extension = extension(path);
    {
        let mut apps = APPS.lock().unwrap_or_else(|e| e.into_inner());
        if apps.iter().any(|(e, _)| *e == extension) {
            return;
        }
        apps.push((extension.clone(), None));
    }
    let window = hwnd.0 as isize;
    std::thread::spawn(move || unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let app = crate::integration::default_app_name(&format!(".{extension}"));
        if com {
            CoUninitialize();
        }
        if let Some(slot) = APPS.lock().unwrap_or_else(|e| e.into_inner()).iter_mut().find(|(e, _)| *e == extension) {
            slot.1 = app;
        }
        let _ = InvalidateRect(Some(HWND(window as *mut _)), None, false);
    });
}

fn current(q: &Quick) -> Option<&Path> {
    q.files.get(q.index).map(PathBuf::as_path)
}

fn open_label(q: &Quick) -> String {
    match current(q).filter(|path| !path.is_dir()).and_then(default_app) {
        Some(app) => format!("Open in {app}"),
        None => "Open".into(),
    }
}

/// Quick view commands, and Markup, which leaves Quick view. Returns true
/// when handled.
pub(super) unsafe fn command(hwnd: HWND, command: Command, keyboard: bool) -> bool {
    let (quick, other) = with_state(|s| (s.quick.is_some(), s.quick.as_ref().is_some_and(|q| q.other.is_some()))).unwrap_or_default();
    if quick {
        settle(hwnd);
    }
    match command {
        Command::OpenInEditor => open_default(hwnd),
        Command::IndexSheet => toggle_grid(hwnd),
        // The editor cannot mark up a type it does not open.
        Command::ToggleMarkup if other => {}
        Command::ToggleMarkup if quick => open_editor(hwnd, true),
        Command::AppMenu if quick => more_menu(hwnd, keyboard),
        _ => return false,
    }
    true
}

/// The Open button: the file's default app, or the editor in this window
/// when that app is this one.
unsafe fn open_default(hwnd: HWND) {
    let Some((path, other)) = with_state(|s| {
        let q = s.quick.as_ref()?;
        Some((current(q)?.to_path_buf(), q.other.is_some()))
    })
    .flatten() else {
        return;
    };
    if !other && default_app(&path).is_none() {
        return open_editor(hwnd, false);
    }
    match crate::integration::open_with_default(Path::new(&display_path(&path))) {
        Ok(()) => {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        Err(error) => sheet::alert(hwnd, "Open", &error),
    }
}

/// More: the commands the header leaves out, so it stays as short as the
/// reference frames' header.
unsafe fn more_menu(hwnd: HWND, keyboard: bool) {
    let Some((ctx, many, elsewhere, other)) = with_state(|s| {
        let q = s.quick.as_ref()?;
        Some((s.ctx(), q.files.len() > 1, current(q).and_then(default_app).is_some(), q.other.is_some()))
    })
    .flatten() else {
        return;
    };
    let mut items = Vec::new();
    if !other {
        let mut markup = MenuItem::for_command(Command::ToggleMarkup, &ctx);
        markup.label = "&Markup".into();
        markup.checked = None;
        items.push(markup);
        if !ctx.pdf {
            items.push(MenuItem::for_command(Command::Rotate, &ctx));
        }
    }
    if many {
        items.push(MenuItem::for_command(Command::IndexSheet, &ctx));
    }
    if elsewhere && !other {
        let mut editor = MenuItem::for_command(Command::OpenInEditor, &ctx);
        editor.pick = Some(Pick::Index(0));
        items.push(editor);
    }
    if items.is_empty() {
        return;
    }
    match actions::popup(hwnd, items, Some(WidgetId::Command(Command::AppMenu)), None, keyboard) {
        Some(Pick::Command(command)) => actions::execute(hwnd, command, keyboard),
        Some(Pick::Index(_)) => open_editor(hwnd, false),
        None => {}
    }
}

/// Right-click: Copy and Open, as in the reference frames.
pub(super) unsafe fn context_menu(hwnd: HWND, at: Option<(f32, f32)>) {
    let Some((ctx, label)) = with_state(|s| Some((s.ctx(), open_label(s.quick.as_ref()?)))).flatten() else {
        return;
    };
    let mut copy = MenuItem::for_command(Command::CopyText, &ctx);
    copy.label = "&Copy".into();
    let mut open = MenuItem::for_command(Command::OpenInEditor, &ctx);
    open.label = format!("&{}", label.replace('&', "&&"));
    if label != "Open" {
        // Enter opens the editor, not the other app.
        open.shortcut.clear();
    }
    let keyboard = at.is_none();
    if let Some(Pick::Command(command)) = actions::popup(hwnd, vec![copy, MenuItem::separator(), open], None, at, keyboard) {
        actions::execute(hwnd, command, keyboard);
    }
}

/// Turns Quick view into the editor in place: same document, page, and zoom.
unsafe fn open_editor(hwnd: HWND, markup: bool) {
    settle(hwnd);
    let Some((title, doc)) = with_state(|s| {
        let has_content = s.frame.is_some() || s.pdf.is_some();
        // Measured while the Quick view layout still applies.
        let ratio = document::ratio(s);
        let doc = s.layout().document;
        s.quick.take()?;
        if has_content && s.zoom != Zoom::Ratio(ratio) {
            s.zoom = Zoom::Ratio(ratio);
        }
        s.tabs = s.displayed.iter().map(|(path, _)| path.clone()).collect();
        s.focus = Some(WidgetId::Document);
        if markup {
            s.set_markup(true);
        }
        Some((s.path.as_deref().map(|p| format!("{} - Preview for Windows", file_name(p))), doc))
    })
    .flatten() else {
        return;
    };
    set_layered(hwnd, false);
    set_style(hwnd, false);
    if !is_full_screen() {
        let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
        let chrome = with_state(|s| s.layout().document.y0).unwrap_or(48.0 * scale);
        // The document keeps its size; the editor's bar goes above it.
        let size = (doc.width().max(EDITOR_WIDTH * scale), doc.height() + chrome);
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
    let Some((grid, closing, chrome_focus, other)) = with_state(|s| {
        let q = s.quick.as_ref().filter(|_| s.sheet.is_none())?;
        let rail = matches!(s.focus, Some(WidgetId::SidebarItem(_)));
        Some((q.grid.as_ref().map(|g| g.focus), q.closing(), s.focus_visible && (rail || strip_has(s, s.focus)), q.other.is_some()))
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
        VK_RETURN if chrome_focus => return false,
        VK_RETURN => match grid {
            Some(focus) => pick(hwnd, focus),
            None if other => open_default(hwnd),
            None => open_editor(hwnd, false),
        },
        VK_TAB => {
            reveal(hwnd);
            return false;
        }
        VK_PRIOR | VK_NEXT | VK_HOME | VK_END if !ctrl && grid.is_none() => {
            with_state(|s| match s.quick.as_mut().and_then(|q| q.other.as_mut()).and_then(|o| o.text.as_mut()) {
                Some(text) => text.key(VIRTUAL_KEY(vk)),
                None => document::key(s, vk, shift),
            });
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

/// The wheel scrolls the index sheet or a text file. Returns true when it did.
pub(super) fn wheel(s: &mut State, delta: f32) -> bool {
    let c = cells(s);
    let Some(q) = s.quick.as_mut() else {
        return false;
    };
    let Some(grid) = q.grid.as_mut() else {
        return q.other.as_mut().and_then(|o| o.text.as_mut()).map(|text| text.wheel(delta)).is_some();
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
    let top = header_height(s);
    let area = Rect { x0: 16.0 * scale, y0: top, x1: (s.size.0 - 16.0 * scale).max(16.0 * scale), y1: s.size.1.max(top) };
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

/// Replaces the editor layout: the header on top, the page rail at the
/// left, and the document below; the index sheet replaces the document.
/// In full screen the document fills the screen and the header floats.
pub(super) fn layout(s: &State, q: &Quick, layout: &mut Layout) {
    let (w, h, scale) = (s.size.0.max(1.0), s.size.1.max(1.0), s.scale);
    let full_screen = is_full_screen();
    let modal = layout.sheet.is_some();
    let grid = q.grid.is_some();
    let header = Rect::new(0.0, 0.0, w, header_height(s).min(h));
    let shown = !full_screen || strip_shown(q, s.animations, Instant::now());
    let rail = rail(s);
    let top = if full_screen { 0.0 } else { header.y1 };
    let left = rail.map_or(0.0, |r| r.x1);
    let mut doc = Rect { x0: left, y0: top, x1: w.max(left), y1: h.max(top) };
    // PDF pages get this space from the page gap; images get it here.
    if !full_screen && !s.is_pdf() {
        doc = doc.inset(document::gap(s));
    }
    layout.widgets.retain(|x| x.region == Region::Sheet || x.id == WidgetId::Document);
    if let Some(document) = layout.widgets.iter_mut().find(|x| x.id == WidgetId::Document) {
        document.rect = match (grid, full_screen && shown) {
            (true, _) => Rect::default(),
            // The floating header takes the clicks above the document.
            (false, true) => Rect { y0: header.y1, ..doc },
            (false, false) => doc,
        };
        document.focusable &= !grid;
    }
    // The header drags the window, except in full screen.
    layout.title_bar = if full_screen { Rect::default() } else { header };
    layout.title_text = Rect::default();
    layout.tab_strip = None;
    layout.markup_bar = None;
    layout.sidebar = rail;
    layout.sidebar_edge = None;
    layout.sidebar_panel = rail.unwrap_or_default();
    layout.sidebar_content = 0.0;
    layout.contact_sheet = false;
    layout.document = doc;
    layout.info_area = doc;
    layout.empty = None;
    layout.toolbar_overflow.clear();
    layout.markup_overflow.clear();
    let mut header_widgets = Vec::new();
    if shown {
        let ctx = s.ctx();
        let (button, gap, pad) = (HEADER_BUTTON * scale, HEADER_GAP * scale, HEADER_PAD * scale);
        let y = ((header.height() - button) / 2.0).round();
        let mut close = plain_widget(WidgetId::Close, Role::Button, Region::TitleBar, Rect::new(pad, y, button, button), "Close".into());
        close.glyph = Some(glyph::CANCEL);
        close.tooltip = "Close (Esc)".into();
        let full = widgets::command_widget(Command::FullScreen, Region::TitleBar, Rect::new(close.rect.x1 + gap, y, button, button), &ctx);
        let label = open_label(q);
        let text = fonts(scale, s.text_scale).map_or(0.0, |f| measure(&label, &f.body, 10_000.0).0);
        let pill_h = (header.height() - 8.0 * scale).max(button);
        let pill_w = (text.ceil() + 2.0 * PILL_PAD * scale).max(button).min(w * 0.45);
        let pill = Rect::new(w - pad - pill_w, ((header.height() - pill_h) / 2.0).round(), pill_w, pill_h);
        let mut open = widgets::command_widget(Command::OpenInEditor, Region::TitleBar, pill, &ctx);
        open.tooltip = if label == "Open" { "Open (Enter)".into() } else { label.clone() };
        open.label = label;
        let share = widgets::command_widget(Command::Share, Region::TitleBar, Rect::new(pill.x0 - gap - button, y, button, button), &ctx);
        let more = widgets::command_widget(Command::AppMenu, Region::TitleBar, Rect::new(share.rect.x0 - gap - button, y, button, button), &ctx);
        let name = full.rect.x1 + 2.0 * gap;
        layout.title_text = Rect { x0: name, y0: header.y0, x1: (more.rect.x0 - 2.0 * gap).max(name), y1: header.y1 };
        header_widgets = vec![close, full, more, share, open];
    }
    for widget in &mut header_widgets {
        widget.focusable &= !modal;
    }
    layout.widgets.splice(0..0, header_widgets);
    if let Some(rail) = rail {
        let current = s.page as usize;
        let mut bottom = rail.y0;
        for (page, r) in rail_rows(s, rail) {
            bottom = r.y1;
            if r.y1 <= rail.y0 || r.y0 >= rail.y1 {
                continue;
            }
            // Clipped to the rail, so a half-hidden row never takes the header's clicks.
            let rect = Rect { x0: rail.x0, y0: r.y0.max(rail.y0), x1: rail.x1, y1: r.y1.min(rail.y1) };
            let mut item = plain_widget(WidgetId::SidebarItem(page), Role::ListItem, Region::Sidebar, rect, format!("Page {}", page + 1));
            item.checked = Some(page == current);
            item.focusable = !modal && page == current;
            layout.widgets.push(item);
        }
        layout.sidebar_content = bottom + RAIL_PAD * scale + s.sidebar_scroll[0] - rail.y0;
    }
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

/// One color behind the header, the rail, and the content, as in the
/// reference frames: the commanding layer over Mica, else solid. White
/// pages need a gray behind them in light mode.
fn tint(t: &Theme) -> Rgba {
    if t.mode == Mode::Light && !t.mica {
        t.chrome
    } else {
        t.bar
    }
}

/// The header's colors at `alpha` of their opacity.
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

/// Draws the window tint, the document or the index sheet, the page rail,
/// then the header. Returns true when the document shows complete content.
pub(super) fn paint(l: &Look, bitmap: Option<&ID2D1Bitmap>, s: &mut State, layout: &Layout) -> bool {
    let full_screen = is_full_screen();
    let tint = tint(&l.t);
    l.p.fill(Rect::new(0.0, 0.0, s.size.0, s.size.1), if full_screen { l.t.canvas } else { tint });
    let grid = s.quick.as_ref().is_some_and(|q| q.grid.is_some());
    let text_scale = s.text_scale;
    let drew = if grid {
        paint_grid(l, s);
        false
    } else if let Some(other) = s.quick.as_mut().and_then(|q| q.other.as_mut()) {
        other.paint(l, layout.document, text_scale);
        false
    } else {
        // Pages and images sit on the window color, with no canvas or halo around them.
        let theme = s.theme;
        s.theme.canvas = Rgba(0.0, 0.0, 0.0, 0.0);
        s.theme.document_halo = Rgba(0.0, 0.0, 0.0, 0.0);
        let drew = document::paint(l.p, bitmap, s, layout.document);
        s.theme = theme;
        drew
    };
    if let Some(rail) = layout.sidebar {
        paint_rail(l, s, rail);
    }
    let Some(q) = s.quick.as_ref() else {
        return drew;
    };
    let alpha = header_alpha(q, s.animations, Instant::now());
    if alpha <= 0.0 {
        return drew;
    }
    if full_screen {
        l.p.fill(Rect::new(0.0, 0.0, s.size.0, header_height(s)), tint.alpha(tint.3 * alpha));
    }
    let look = Look { p: l.p, t: faded(l.t, alpha), f: l.f, s: l.s };
    let name = current(q).map(file_name).unwrap_or_default();
    l.p.text(&name, layout.title_text, &l.f.strong, look.t.text, Align::Leading);
    // Header icons are quieter than the name, as in the reference frames.
    let icons = Look { p: l.p, t: Theme { text: look.t.text_secondary, ..look.t }, f: l.f, s: l.s };
    for w in layout.widgets.iter().filter(|w| w.region == Region::TitleBar) {
        if w.id == WidgetId::Command(Command::OpenInEditor) {
            pill(&look, s, w);
        } else {
            icon_button(&icons, s, w);
        }
    }
    drew
}

/// The Open button: a quiet filled capsule, as in the reference frames.
fn pill(l: &Look, s: &State, w: &Widget) {
    if l.t.mode == Mode::Contrast {
        return text_button(l, s, w);
    }
    let (r, radius) = (w.rect, 6.0 * l.s);
    l.p.fill_round(r, radius, l.t.hover);
    if s.pressed == Some(w.id) {
        l.p.fill_round(r, radius, l.t.pressed);
    } else if s.hover == Some(w.id) {
        l.p.fill_round(r, radius, l.t.hover);
    }
    l.p.text(&w.label, r.inset(4.0 * l.s), &l.f.body, l.t.text, Align::Center);
}

/// Page thumbnails from the editor's thumbnail renders; the current page
/// has the accent frame.
fn paint_rail(l: &Look, s: &mut State, rail: Rect) {
    let Some(doc) = s.pdf.as_ref().map(|v| v.doc) else {
        return;
    };
    let rows: Vec<(usize, Rect)> = rail_rows(s, rail).filter(|(_, r)| r.y1 > rail.y0 && r.y0 < rail.y1).collect();
    let (current, hover, sc) = (s.page as usize, s.hover, l.s);
    l.p.push_clip(rail);
    for (page, r) in rows {
        if hover == Some(WidgetId::SidebarItem(page)) {
            l.p.fill_round(r.inset(-4.0 * sc), 6.0 * sc, l.t.hover);
        }
        l.p.fill(r, Rgba(1.0, 1.0, 1.0, 1.0));
        if let Some(tile) = s.cache.get(&Key { doc, work: Work::Thumb { page: page as u32 } }) {
            l.p.draw_bitmap(&tile.bitmap, r);
        }
        if page == current {
            l.p.stroke_round(r.inset(-3.0 * sc), 3.0 * sc, l.t.accent, 2.0 * sc);
        } else {
            l.p.stroke_round(r.inset(-sc), 0.0, l.t.image_outline, sc);
        }
    }
    l.p.pop_clip();
}

fn paint_grid(l: &Look, s: &mut State) {
    let (scale, ts) = (l.s, s.text_scale);
    let area = cells(s).area;
    let cells = visible_cells(s);
    let hover = s.hover;
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
    let factory: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(display_path(path)), None).ok()?;
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
