//! The main window: creation, the message loop, the custom title bar,
//! pointer and keyboard input, and live theme, DPI, and text-size changes.
use super::{
    a11y, actions,
    app::{close_tab, install, invalidate, select_tab, tick, uninstall, with_state, State},
    commands::{self, Chord, Command},
    document::{self, Phase, PointerEvent, PointerKind},
    paint, sheet, sidebar, theme,
    widgets::{self, Layout, Scope, WidgetId},
    worker::{Workers, WM_APP_WAKE},
};
use std::{
    cell::RefCell,

    path::PathBuf,
    time::{Duration, Instant},
};
use windows::{
    core::*,
    UI::ViewManagement::UISettings,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::{LibraryLoader::GetModuleHandleW, Ole::{OleInitialize, OleUninitialize}},
        UI::{
            HiDpi::*,
            Input::{KeyboardAndMouse::*, Pointer::*},
            Shell::{DragFinish, HDROP},
            WindowsAndMessaging::*,
        },
    },
};

/// W1-D's single-instance handoff finds the window by this class name.
pub(super) const CLASS: PCWSTR = w!("PreviewForWindowsMain");
const WM_APP_TEXT_SCALE: u32 = WM_APP + 3;

thread_local! {
    static SETTINGS: RefCell<Option<UISettings>> = const { RefCell::new(None) };
}

pub fn run() -> Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // Mouse input arrives as WM_POINTER, like pen and touch. It is
        // process-wide and must come before any window exists.
        // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-enablemouseinpointer
        let _ = EnableMouseInPointer(true);
        // OLE, not only COM: drag-out and the share sheet (W1-D) need it.
        // https://learn.microsoft.com/windows/win32/api/ole2/nf-ole2-oleinitialize
        OleInitialize(None)?;
        let mut paths = Vec::new();
        for path in std::env::args_os().skip(1).map(PathBuf::from).map(|p| std::fs::canonicalize(&p).unwrap_or(p)) {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        let instance = GetModuleHandleW(None)?;
        let wc = WNDCLASSW {
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: instance.into(),
            lpszClassName: CLASS,
            lpfnWndProc: Some(wndproc),
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            return Err(Error::from_thread());
        }
        let system = GetDpiForSystem() as f32 / 96.0;
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            CLASS,
            w!("Preview for Windows"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            (1100.0 * system) as i32,
            (800.0 * system) as i32,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        let workers = Workers::start(hwnd).map_err(|_| Error::from_thread())?;
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let mut state = State::new(
            workers,
            &paths,
            (client.right as f32, client.bottom as f32),
            GetDpiForWindow(hwnd) as f32 / 96.0,
            theme::text_scale_from_registry(),
            theme::apply(hwnd, theme::current()),
        );
        state.bench = super::bench::Bench::from_env();
        install(state);
        // Apply WM_NCCALCSIZE now that the state exists, so the caption goes.
        let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        super::drop::register(hwnd);
        SetTimer(Some(hwnd), 1, 10, None);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);
        let mut message = MSG::default();
        loop {
            let result = GetMessageW(&mut message, None, 0, 0).0;
            if result == -1 {
                return Err(Error::from_thread());
            }
            if result == 0 {
                break;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        uninstall();
        OleUninitialize();
        Ok(())
    }
}

/// Starts UISettings after first content: it is the documented source of
/// the text size setting and raises TextScaleFactorChanged.
/// https://learn.microsoft.com/uwp/api/windows.ui.viewmanagement.uisettings.textscalefactor
pub(super) unsafe fn watch_text_scale(hwnd: HWND) {
    let Ok(settings) = UISettings::new() else {
        return;
    };
    let window = hwnd.0 as isize;
    let _ = settings.TextScaleFactorChanged(&windows::Foundation::TypedEventHandler::new(move |_, _| {
        let _ = PostMessageW(Some(HWND(window as *mut _)), WM_APP_TEXT_SCALE, WPARAM(0), LPARAM(0));
        Ok(())
    }));
    SETTINGS.with(|cell| *cell.borrow_mut() = Some(settings));
    text_scale_changed(hwnd);
}

unsafe fn text_scale_changed(hwnd: HWND) {
    let Some(scale) = SETTINGS.with(|cell| cell.borrow().as_ref().and_then(|s| s.TextScaleFactor().ok())) else {
        return;
    };
    let scale = (scale as f32).clamp(1.0, 2.25);
    let changed = with_state(|s| {
        let changed = (s.text_scale - scale).abs() > f32::EPSILON;
        if changed {
            s.text_scale = scale;
            if s.frame.is_some() || s.pdf.is_some() {
                s.due = Some(Instant::now() + Duration::from_millis(120));
            }
        }
        changed
    });
    if changed == Some(true) {
        sheet::position_fields(hwnd);
        invalidate(hwnd);
    }
}

/// Resize border height. Maximized windows extend past the monitor by this.
unsafe fn frame_thickness(hwnd: HWND) -> i32 {
    let dpi = GetDpiForWindow(hwnd);
    GetSystemMetricsForDpi(SM_CYSIZEFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
}

/// Removes the caption but keeps the side and bottom frames, so resizing,
/// snapping, and shadows stay native.
/// https://learn.microsoft.com/windows/win32/dwm/customframe
unsafe fn nc_calc_size(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    let params = &mut *(lparam.0 as *mut NCCALCSIZE_PARAMS);
    let top = params.rgrc[0].top;
    DefWindowProcW(hwnd, WM_NCCALCSIZE, WPARAM(1), lparam);
    params.rgrc[0].top = top + if IsZoomed(hwnd).as_bool() { frame_thickness(hwnd) } else { 0 };
    LRESULT(0)
}

/// Maps a client point to a hit-test code. Returning HTMAXBUTTON over the
/// maximize button turns on Windows 11 snap layouts.
/// https://learn.microsoft.com/windows/apps/desktop/modernize/ui/apply-snap-layout-menu
pub(super) fn hit_code(layout: &Layout, x: f32, y: f32, maximized: bool, border: f32) -> u32 {
    if !maximized && y < border {
        return HTTOP;
    }
    if let Some(w) = widgets::hit(&layout.widgets, x, y) {
        return match w.id {
            WidgetId::Minimize => HTMINBUTTON,
            WidgetId::Maximize => HTMAXBUTTON,
            WidgetId::Close => HTCLOSE,
            _ => HTCLIENT,
        };
    }
    if layout.title_bar.contains(x, y) {
        HTCAPTION
    } else {
        HTCLIENT
    }
}

unsafe fn nc_hit_test(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let hit = DefWindowProcW(hwnd, WM_NCHITTEST, wparam, lparam);
    if hit.0 != HTCLIENT as isize {
        return hit;
    }
    let mut point = lparam_point(lparam);
    let _ = ScreenToClient(hwnd, &mut point);
    let border = frame_thickness(hwnd) as f32;
    let code = with_state(|s| hit_code(&s.layout(), point.x as f32, point.y as f32, s.maximized, border));
    LRESULT(code.unwrap_or(HTCLIENT) as isize)
}

/// Signed screen coordinates packed in an LPARAM. Negative values occur on
/// monitors left of or above the primary one.
fn lparam_point(lparam: LPARAM) -> POINT {
    POINT { x: (lparam.0 as u16 as i16) as i32, y: ((lparam.0 >> 16) as u16 as i16) as i32 }
}

fn caption_widget(code: u32) -> Option<WidgetId> {
    match code {
        HTMINBUTTON => Some(WidgetId::Minimize),
        HTMAXBUTTON => Some(WidgetId::Maximize),
        HTCLOSE => Some(WidgetId::Close),
        _ => None,
    }
}

unsafe fn caption_command(hwnd: HWND, id: WidgetId) {
    let command = match id {
        WidgetId::Minimize => SC_MINIMIZE,
        WidgetId::Maximize if IsZoomed(hwnd).as_bool() => SC_RESTORE,
        WidgetId::Maximize => SC_MAXIMIZE,
        _ => SC_CLOSE,
    };
    let _ = PostMessageW(Some(hwnd), WM_SYSCOMMAND, WPARAM(command as usize), LPARAM(0));
}

/// Caption buttons draw their own hover and press, because DefWindowProc
/// would draw classic buttons on Windows 10.
unsafe fn caption_input(hwnd: HWND, code: u32, phase: Phase) -> bool {
    let target = caption_widget(code);
    match phase {
        Phase::Move => {
            let changed = with_state(|s| std::mem::replace(&mut s.caption_hover, target) != target).unwrap_or(false);
            if target.is_some() {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE | TME_NONCLIENT,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut track);
            }
            if changed {
                invalidate(hwnd);
            }
            false
        }
        Phase::Down => {
            let Some(target) = target else {
                return false;
            };
            with_state(|s| s.caption_pressed = Some(target));
            invalidate(hwnd);
            true
        }
        Phase::Up | Phase::Cancel => {
            let pressed = with_state(|s| s.caption_pressed.take()).flatten();
            invalidate(hwnd);
            if phase == Phase::Up && pressed.is_some() && pressed == target {
                caption_command(hwnd, pressed.unwrap());
            }
            target.is_some()
        }
    }
}

/// Runs a widget's action. Click, Enter, Space, access keys, and UI
/// Automation Invoke all come here.
pub(super) unsafe fn activate(hwnd: HWND, id: WidgetId, keyboard: bool) {
    match id {
        WidgetId::Tab(index) => select_tab(hwnd, index),
        WidgetId::TabClose(index) => close_tab(hwnd, index),
        WidgetId::NewTab => actions::execute(hwnd, Command::Open, keyboard),
        WidgetId::Command(command) => actions::execute(hwnd, command, keyboard),
        WidgetId::Minimize | WidgetId::Maximize | WidgetId::Close => caption_command(hwnd, id),
        WidgetId::SidebarTab(index) => {
            with_state(|s| {
                s.sidebar_tab = index;
                sidebar::follow_page(s);
            });
        }
        WidgetId::SidebarItem(index) => {
            with_state(|s| sidebar::activate(s, index));
        }
        WidgetId::SheetButton(index) => sheet::finish(Some(index)),
        WidgetId::SheetField(index) => {
            let edit = with_state(|s| s.sheet.as_ref().and_then(|x| x.fields.get(index)).map(|f| f.edit)).flatten();
            if let Some(edit) = edit {
                let _ = SetFocus(Some(edit));
            }
        }
        WidgetId::Document => {
            with_state(|s| s.focus = Some(WidgetId::Document));
        }
    }
    invalidate(hwnd);
}

unsafe fn read_pointer(hwnd: HWND, message: u32, wparam: WPARAM) -> Option<(PointerEvent, bool)> {
    let id = (wparam.0 & 0xffff) as u32;
    let mut info = POINTER_INFO::default();
    GetPointerInfo(id, &mut info).ok()?;
    let kind = match info.pointerType {
        PT_PEN => PointerKind::Pen,
        PT_TOUCH => PointerKind::Touch,
        _ => PointerKind::Mouse,
    };
    let (mut pressure, mut eraser) = (None, false);
    if kind == PointerKind::Pen {
        let mut pen = POINTER_PEN_INFO::default();
        if GetPointerPenInfo(id, &mut pen).is_ok() {
            // POINTER_PEN_INFO.pressure runs from 0 to 1024.
            // https://learn.microsoft.com/windows/win32/api/winuser/ns-winuser-pointer_pen_info
            pressure = Some(pen.pressure.min(1024) as f32 / 1024.0);
            eraser = pen.penFlags & PEN_FLAG_ERASER != 0;
        }
    }
    let mut point = info.ptPixelLocation;
    let _ = ScreenToClient(hwnd, &mut point);
    let change = info.ButtonChangeType;
    let phase = if info.pointerFlags.contains(POINTER_FLAG_CANCELED) {
        Phase::Cancel
    } else if change == POINTER_CHANGE_FIRSTBUTTON_DOWN {
        Phase::Down
    } else if change == POINTER_CHANGE_FIRSTBUTTON_UP || (message == WM_POINTERUP && kind != PointerKind::Mouse) {
        Phase::Up
    } else {
        Phase::Move
    };
    let event = PointerEvent {
        id,
        kind,
        phase,
        x: point.x as f32,
        y: point.y as f32,
        pressure,
        contact: info.pointerFlags.contains(POINTER_FLAG_INCONTACT) || info.pointerFlags.contains(POINTER_FLAG_FIRSTBUTTON),
        eraser,
    };
    Some((event, change == POINTER_CHANGE_SECONDBUTTON_UP))
}

/// Routes one pointer event to the sheet, a widget, or the document.
unsafe fn pointer(hwnd: HWND, e: PointerEvent, secondary_up: bool) {
    if secondary_up {
        actions::context_menu(hwnd, Some((e.x, e.y)));
        return;
    }
    if super::organize::pointer(hwnd, &e) {
        return;
    }
    let mut activate_id = None;
    let mut to_document = false;
    let mut repaint = false;
    with_state(|s| {
        if s.caption_hover.take().is_some() {
            repaint = true;
        }
        let layout = s.layout();
        let over = widgets::hit(&layout.widgets, e.x, e.y).filter(|w| w.enabled).map(|w| (w.id, !w.tooltip.is_empty()));
        let on_document = s.sheet.is_none() && matches!(over, Some((WidgetId::Document, _)));
        let tracking = s.drag.is_some() || s.pinch.is_some() || s.touches.iter().any(|(id, _)| *id == e.id);
        match e.phase {
            Phase::Down => {
                s.focus_visible = false;
                s.keytips = None;
                s.tooltip = None;
                s.hover_since = None;
                if on_document {
                    s.focus = Some(WidgetId::Document);
                    to_document = true;
                } else if let Some((id, _)) = over {
                    s.pressed = Some(id);
                    if e.kind == PointerKind::Mouse {
                        SetCapture(hwnd);
                    }
                }
                repaint = true;
            }
            Phase::Move => {
                if tracking {
                    to_document = true;
                } else if on_document {
                    to_document = true;
                } else {
                    let hover = over.filter(|(id, _)| *id != WidgetId::Document);
                    let tooltip = hover.is_some_and(|(_, t)| t);
                    if s.set_hover(hover.map(|(id, _)| id)) {
                        repaint = true;
                    }
                    if !tooltip {
                        s.hover_since = None;
                    }
                }
            }
            Phase::Up | Phase::Cancel => {
                if tracking {
                    to_document = true;
                }
                if let Some(pressed) = s.pressed.take() {
                    let _ = ReleaseCapture();
                    repaint = true;
                    if e.phase == Phase::Up && over.map(|(id, _)| id) == Some(pressed) {
                        activate_id = Some(pressed);
                    }
                }
            }
        }
    });
    if to_document && e.phase == Phase::Down {
        let edit_path = with_state(|s| {
            (s.crop || s.markup.is_some()).then(|| s.path.clone()).flatten()
        })
        .flatten();
        if let Some(path) = edit_path {
            if !actions::prepare_save(hwnd, &path) {
                return;
            }
        }
    }
    if to_document && with_state(|s| document::on_pointer(hwnd, s, &e)).unwrap_or(false) {
        repaint = true;
    }
    if let Some(id) = activate_id {
        activate(hwnd, id, false);
    }
    if repaint {
        invalidate(hwnd);
    }
}

/// The wheel scrolls the sidebar list or the document under the pointer;
/// Ctrl+wheel zooms around the pointer.
unsafe fn wheel(hwnd: HWND, wparam: WPARAM, lparam: LPARAM, horizontal: bool) {
    let delta = ((wparam.0 >> 16) as u16 as i16) as f32;
    let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
    let mut point = lparam_point(lparam);
    let _ = ScreenToClient(hwnd, &mut point);
    let at = (point.x as f32, point.y as f32);
    with_state(|s| {
        if s.sheet.is_some() {
            return;
        }
        let layout = s.layout();
        if !ctrl && !horizontal && layout.sidebar.is_some_and(|r| r.contains(at.0, at.1)) {
            sidebar::wheel(s, delta, &layout);
        } else {
            document::wheel(s, delta, horizontal, ctrl, at);
        }
    });
    invalidate(hwnd);
}

fn key_char(vk: u16) -> Option<char> {
    matches!(vk, 0x30..=0x39 | 0x41..=0x5A).then(|| vk as u8 as char)
}

/// Escape leaves modes and cancels long jobs, as in the pre-split shell.
unsafe fn escape(hwnd: HWND) {
    with_state(|s| {
        s.crop = false;
        s.zoom_select = false;
        s.slideshow = None;
        if let Some(cancel) = &s.cancel {
            cancel.store(true, std::sync::atomic::Ordering::Release);
        }
        s.markup = None;
        s.ink.clear();
        s.signature = None;
        s.selection = None;
        s.drag = None;
        s.organize.cancel();
        s.tooltip = None;
    });
    let _ = ReleaseCapture();
    invalidate(hwnd);
}

/// Returns true when the key was handled.
unsafe fn key_down(hwnd: HWND, vk: u16, system: bool) -> bool {
    let down = |key: VIRTUAL_KEY| GetKeyState(key.0 as i32) < 0;
    let (ctrl, shift, alt) = (down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU));
    if vk == VK_MENU.0 {
        with_state(|s| s.alt_armed = true);
        return true;
    }
    with_state(|s| {
        s.alt_armed = false;
        s.tooltip = None;
    });
    // Alt+F4 and Alt+Space belong to Windows.
    if alt && matches!(VIRTUAL_KEY(vk), VK_F4 | VK_SPACE) {
        return false;
    }
    if with_state(|s| s.sheet.is_some()).unwrap_or(false) {
        let focus = with_state(|s| s.focus).flatten();
        match VIRTUAL_KEY(vk) {
            VK_TAB => sheet::move_focus(hwnd, shift),
            VK_ESCAPE => sheet::finish(None),
            VK_RETURN => sheet::finish(Some(match focus {
                Some(WidgetId::SheetButton(i)) => i,
                _ => 0,
            })),
            VK_SPACE => {
                if let Some(WidgetId::SheetButton(i)) = focus {
                    sheet::finish(Some(i));
                }
            }
            _ => {}
        }
        return true;
    }
    // Keytips: Alt shows them; a letter runs that button. Alt+letter works
    // directly. https://learn.microsoft.com/windows/apps/design/input/access-keys
    let scope = with_state(|s| s.keytips).flatten().or((system && alt).then_some(Scope::Root));
    if let Some(scope) = scope {
        if vk == VK_ESCAPE.0 {
            with_state(|s| s.keytips = (scope == Scope::Markup).then_some(Scope::Root));
            invalidate(hwnd);
            return true;
        }
        if let Some(key) = key_char(vk) {
            let target = with_state(|s| {
                s.keytips = None;
                widgets::access_key_target(&s.layout().widgets, scope, key)
            })
            .flatten();
            match target {
                Some(WidgetId::Command(Command::ToggleMarkup)) if scope == Scope::Root => {
                    // Alt, M opens the markup bar and its scope (Alt, M, H).
                    with_state(|s| {
                        s.set_markup(true);
                        s.keytips = Some(Scope::Markup);
                    });
                }
                Some(id) => activate(hwnd, id, true),
                None => {}
            }
            invalidate(hwnd);
            return true;
        }
        with_state(|s| s.keytips = None);
        invalidate(hwnd);
    }
    if vk == VK_TAB.0 && !ctrl && !alt {
        with_state(|s| {
            let layout = s.layout();
            s.focus = widgets::next_focus(&layout.widgets, s.focus, shift);
            s.focus_visible = true;
        });
        invalidate(hwnd);
        return true;
    }
    if matches!(VIRTUAL_KEY(vk), VK_RETURN | VK_SPACE) && !ctrl && !alt {
        let focus = with_state(|s| s.focus.filter(|f| *f != WidgetId::Document && s.focus_visible)).flatten();
        if let Some(id) = focus {
            activate(hwnd, id, true);
            return true;
        }
    }
    if ctrl && !alt && vk == 0x41 {
        let handled = with_state(|s| {
            let pages = text_page_count(s.pdf.as_ref().map(|pdf| pdf.sizes.len() as u32), s.frame.is_some());
            if !matches!(s.focus, Some(WidgetId::Document) | None) || pages == 0 {
                return false;
            }
            s.text_selection = Some(super::text::all(pages));
            true
        });
        if handled == Some(true) {
            invalidate(hwnd);
            return true;
        }
    }
    // Scroll keys go to the focused sidebar list or the document.
    if !ctrl && !alt {
        let handled = with_state(|s| match s.focus {
            Some(WidgetId::SidebarItem(index)) => sidebar::key(s, vk, index),
            Some(WidgetId::Document) | None => document::text_key(s, vk, shift) || document::key(s, vk, shift),
            _ => false,
        });
        if handled == Some(true) {
            invalidate(hwnd);
            return true;
        }
    }
    if let Some(command) = commands::lookup(Chord { key: vk, ctrl, shift, alt }) {
        if matches!(command, Command::NextPane | Command::PreviousPane) {
            with_state(|s| s.focus_visible = true);
        }
        actions::execute(hwnd, command, true);
        return true;
    }
    if vk == VK_ESCAPE.0 {
        escape(hwnd);
        return true;
    }
    false
}

fn text_page_count(pdf_pages: Option<u32>, has_image: bool) -> u32 {
    pdf_pages.unwrap_or_else(|| u32::from(has_image))
}

/// Settings broadcasts are rare, so the theme is simply read and applied again.
unsafe fn retheme(hwnd: HWND) {
    let theme = theme::apply(hwnd, theme::current());
    let edits = with_state(|s| {
        s.theme = theme;
        s.animations = theme::animations_enabled();
        s.sheet.as_ref().map(|x| x.fields.iter().map(|f| f.edit).collect::<Vec<_>>()).unwrap_or_default()
    })
    .unwrap_or_default();
    for edit in edits {
        let _ = InvalidateRect(Some(edit), None, true);
    }
    invalidate(hwnd);
}

unsafe fn drop_files(hwnd: HWND, wparam: WPARAM) {
    let drop = HDROP(wparam.0 as *mut _);
    let paths = super::drop::hdrop_paths(drop);
    DragFinish(drop);
    super::drop::finish(hwnd, paths, None);
}

unsafe fn close(hwnd: HWND) {
    if with_state(|s| s.sheet.is_some()).unwrap_or(false) {
        // Closing while a sheet is open cancels the sheet first.
        sheet::finish(None);
        return;
    }
    with_state(|s| s.pause_save_dispatch(true));
    let (dirty, saving) = with_state(|s| s.window_close_state()).unwrap_or((false, false));
    if saving {
        sheet::alert(hwnd, "Saving", "Wait for the save to finish, then close Preview.");
        with_state(|s| s.pause_save_dispatch(false));
        return;
    }
    if dirty
        && !sheet::confirm(
            hwnd,
            "Close Preview?",
            "Some edits are not saved. Closing now discards those edits.",
            "Close without saving",
        )
    {
        with_state(|s| s.pause_save_dispatch(false));
        return;
    }
    if with_state(|s| s.window_close_state().1).unwrap_or(true) {
        sheet::alert(hwnd, "Saving", "A file started saving. Wait for it to finish, then close Preview.");
        with_state(|s| s.pause_save_dispatch(false));
        return;
    }
    with_state(|s| s.cleanup_snapshots());
    let _ = DestroyWindow(hwnd);
}

fn session_end_allowed((dirty, saving): (bool, bool)) -> bool {
    !dirty && !saving
}

unsafe extern "system" fn wndproc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_NCCALCSIZE if wparam.0 != 0 => nc_calc_size(hwnd, lparam),
        WM_NCHITTEST => nc_hit_test(hwnd, wparam, lparam),
        WM_NCPOINTERUPDATE | WM_NCPOINTERDOWN | WM_NCPOINTERUP => {
            // HIWORD(wParam) is the WM_NCHITTEST result for this point.
            let code = ((wparam.0 >> 16) & 0xffff) as u32;
            let phase = match message {
                WM_NCPOINTERDOWN => Phase::Down,
                WM_NCPOINTERUP => Phase::Up,
                _ => Phase::Move,
            };
            if caption_input(hwnd, code, phase) {
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_NCMOUSEMOVE => {
            caption_input(hwnd, wparam.0 as u32, Phase::Move);
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if caption_widget(wparam.0 as u32).is_some() => {
            caption_input(hwnd, wparam.0 as u32, Phase::Down);
            LRESULT(0)
        }
        WM_NCLBUTTONUP if caption_widget(wparam.0 as u32).is_some() => {
            caption_input(hwnd, wparam.0 as u32, Phase::Up);
            LRESULT(0)
        }
        WM_NCMOUSELEAVE => {
            with_state(|s| {
                s.caption_hover = None;
                s.caption_pressed = None;
            });
            invalidate(hwnd);
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_ACTIVATE => {
            with_state(|s| s.active = (wparam.0 & 0xffff) as u32 != WA_INACTIVE);
            invalidate(hwnd);
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_SETFOCUS => {
            a11y::focus_changed(true);
            invalidate(hwnd);
            LRESULT(0)
        }
        WM_KILLFOCUS => {
            a11y::focus_changed(false);
            with_state(|s| {
                s.alt_armed = false;
                s.keytips = None;
            });
            invalidate(hwnd);
            LRESULT(0)
        }
        WM_GETOBJECT => a11y::get_object(wparam, lparam).unwrap_or_else(|| DefWindowProcW(hwnd, message, wparam, lparam)),
        WM_SIZE => {
            let (width, height) = ((lparam.0 & 0xffff) as f32, ((lparam.0 >> 16) & 0xffff) as f32);
            with_state(|s| {
                s.size = (width, height);
                s.maximized = wparam.0 as u32 == SIZE_MAXIMIZED;
                if let Some(renderer) = &s.renderer {
                    if renderer.resize(width as u32, height as u32).is_err() {
                        s.renderer = None;
                    }
                }
                let shown = s.frame.is_some() || s.pdf.is_some();
                s.due = Some(Instant::now() + if shown { document::SETTLE } else { Duration::ZERO });
            });
            sheet::position_fields(hwnd);
            invalidate(hwnd);
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let info = &mut *(lparam.0 as *mut MINMAXINFO);
            let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
            info.ptMinTrackSize = POINT { x: (360.0 * scale) as i32, y: (320.0 * scale) as i32 };
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let rect = &*(lparam.0 as *const RECT);
            with_state(|s| s.scale = ((wparam.0 >> 16) & 0xffff) as f32 / 96.0);
            let _ = SetWindowPos(
                hwnd,
                None,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            sheet::position_fields(hwnd);
            invalidate(hwnd);
            LRESULT(0)
        }
        // Theme, contrast, and color changes arrive here; re-read and repaint.
        // https://learn.microsoft.com/windows/apps/desktop/modernize/ui/apply-windows-themes
        WM_SETTINGCHANGE | WM_THEMECHANGED | WM_SYSCOLORCHANGE => {
            retheme(hwnd);
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            BeginPaint(hwnd, &mut ps);
            paint::paint(hwnd);
            let _ = EndPaint(hwnd, &ps);
            a11y::update();
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_SETCURSOR if (lparam.0 as u16) as i32 == HTCLIENT as i32 => {
            let mut point = POINT::default();
            let text = GetCursorPos(&mut point).is_ok() && ScreenToClient(hwnd, &mut point).as_bool() && with_state(|s| {
                document::text_point(s, point.x as f32, point.y as f32).is_some_and(|(page, at, size)| {
                    s.text_layers.get(&page).is_some_and(|layer| super::text::over_text(layer, at, size))
                })
            }) == Some(true);
            if text {
                let _ = SetCursor(Some(LoadCursorW(None, IDC_IBEAM).unwrap_or_default()));
                LRESULT(1)
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP => {
            match read_pointer(hwnd, message, wparam) {
                Some((event, secondary_up)) => {
                    pointer(hwnd, event, secondary_up);
                    LRESULT(0)
                }
                None => DefWindowProcW(hwnd, message, wparam, lparam),
            }
        }
        WM_POINTERLEAVE => {
            if with_state(|s| s.set_hover(None)).unwrap_or(false) {
                invalidate(hwnd);
            }
            LRESULT(0)
        }
        WM_POINTERWHEEL | WM_POINTERHWHEEL => {
            wheel(hwnd, wparam, lparam, message == WM_POINTERHWHEEL);
            LRESULT(0)
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            if key_down(hwnd, wparam.0 as u16, message == WM_SYSKEYDOWN) {
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_SYSKEYUP if wparam.0 as u16 == VK_MENU.0 => {
            // A lone Alt press shows or hides keytips. Alt never enters the
            // system menu here; Alt+Space and F10 still do.
            if with_state(|s| std::mem::take(&mut s.alt_armed)).unwrap_or(false) {
                with_state(|s| s.keytips = if s.keytips.is_some() { None } else { Some(Scope::Root) });
                invalidate(hwnd);
            }
            LRESULT(0)
        }
        // Alt+letter is handled in WM_SYSKEYDOWN; only Alt+Space opens the
        // system menu. Other WM_SYSCHAR messages would beep.
        WM_SYSCHAR if wparam.0 != ' ' as usize => LRESULT(0),
        WM_CONTEXTMENU => {
            // The keyboard (Shift+F10 or the menu key) sends (-1, -1).
            // https://learn.microsoft.com/windows/win32/menurc/wm-contextmenu
            let mut point = lparam_point(lparam);
            if (point.x, point.y) == (-1, -1) {
                actions::context_menu(hwnd, None);
            } else {
                let _ = ScreenToClient(hwnd, &mut point);
                actions::context_menu(hwnd, Some((point.x as f32, point.y as f32)));
            }
            LRESULT(0)
        }
        WM_CTLCOLOREDIT => {
            let theme = with_state(|s| s.theme).unwrap_or_else(|| theme::palette(theme::Mode::Light));
            sheet::color_edit(HDC(wparam.0 as *mut _), theme.text.colorref(), theme.field.colorref())
        }
        WM_COMMAND => {
            // EN_SETFOCUS and EN_KILLFOCUS move the focus line on text boxes.
            let code = ((wparam.0 >> 16) & 0xffff) as u32;
            if code == EN_SETFOCUS {
                if let Some(index) = sheet::field_index(HWND(lparam.0 as *mut _)) {
                    with_state(|s| s.focus = Some(WidgetId::SheetField(index)));
                }
            }
            if code == EN_SETFOCUS || code == EN_KILLFOCUS {
                invalidate(hwnd);
            }
            LRESULT(0)
        }
        WM_DROPFILES => {
            drop_files(hwnd, wparam);
            LRESULT(0)
        }
        WM_TIMER | WM_APP_WAKE => {
            tick(hwnd);
            LRESULT(0)
        }
        a11y::WM_APP_A11Y => {
            if let Some(id) = a11y::widget_for(lparam.0 as u64) {
                if wparam.0 == 0 {
                    activate(hwnd, id, true);
                } else {
                    with_state(|s| {
                        s.focus = Some(id);
                        s.focus_visible = true;
                    });
                    invalidate(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_APP_TEXT_SCALE => {
            text_scale_changed(hwnd);
            LRESULT(0)
        }
        WM_CLOSE => {
            close(hwnd);
            LRESULT(0)
        }
        WM_QUERYENDSESSION => {
            let allowed = with_state(|s| session_end_allowed(s.window_close_state())).unwrap_or(true);
            LRESULT(allowed as isize)
        }
        WM_ENDSESSION if wparam.0 != 0 => {
            with_state(|s| {
                if session_end_allowed(s.window_close_state()) {
                    s.cleanup_snapshots();
                }
            });
            LRESULT(0)
        }
        WM_DESTROY => {
            super::drop::revoke(hwnd);
            let _ = KillTimer(Some(hwnd), 1);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_ocr_has_one_selectable_text_page_even_for_multiframe_images() {
        assert_eq!(text_page_count(None, true), 1);
        assert_eq!(text_page_count(Some(12), true), 12);
        assert_eq!(text_page_count(None, false), 0);
    }
    use crate::ui::commands::Ctx;

    #[test]
    fn windows_session_end_is_allowed_only_when_every_revision_is_safe() {
        assert!(session_end_allowed((false, false)));
        assert!(!session_end_allowed((true, false)), "dirty edits block logoff or restart");
        assert!(!session_end_allowed((false, true)), "an in-flight save blocks logoff or restart");
    }

    fn layout(maximized: bool) -> Layout {
        let tabs = vec!["a.pdf".to_string()];
        widgets::layout(&widgets::Input {
            width: 1000.0,
            height: 700.0,
            scale: 1.0,
            text_scale: 1.0,
            tabs: &tabs,
            active_tab: Some(0),
            maximized,
            has_document: true,
            title: "a.pdf",
            sidebar_open: false,
            sidebar_tab: 0,
            sidebar_list: widgets::SidebarList::Message(""),
            sidebar_scroll: 0.0,
            sidebar_active: 0,
            markup: 0.0,
            ctx: Ctx { has_frame: true, tabs: 1, ..Default::default() },
            sheet: None,
        })
    }

    #[test]
    fn hit_test_maps_caption_buttons_tabs_and_drag_space() {
        let l = layout(false);
        let max = l.widgets.iter().find(|w| w.id == WidgetId::Maximize).unwrap().rect;
        assert_eq!(hit_code(&l, max.x0 + 5.0, 20.0, false, 8.0), HTMAXBUTTON);
        assert_eq!(hit_code(&l, 995.0, 20.0, false, 8.0), HTCLOSE);
        assert_eq!(hit_code(&l, max.x0 - 20.0, 20.0, false, 8.0), HTMINBUTTON);
        assert_eq!(hit_code(&l, 30.0, 20.0, false, 8.0), HTCLIENT, "a tab");
        assert_eq!(hit_code(&l, 600.0, 20.0, false, 8.0), HTCAPTION, "empty title bar drags");
        assert_eq!(hit_code(&l, 600.0, 3.0, false, 8.0), HTTOP, "top resize border");
        assert_eq!(hit_code(&l, 600.0, 3.0, true, 8.0), HTCAPTION, "no resize border when maximized");
        assert_eq!(hit_code(&l, 500.0, 400.0, false, 8.0), HTCLIENT);
    }

    #[test]
    fn signed_coordinates_survive_negative_positions() {
        assert_eq!(lparam_point(LPARAM(((20u32 << 16) | (-12i16 as u16 as u32)) as isize)), POINT { x: -12, y: 20 });
        assert_eq!(lparam_point(LPARAM(0xFFFF_FFFF)), POINT { x: -1, y: -1 });
    }

    #[test]
    fn letters_and_digits_are_access_keys() {
        assert_eq!(key_char(0x41), Some('A'));
        assert_eq!(key_char(0x39), Some('9'));
        assert_eq!(key_char(0x20), None);
        assert_eq!(key_char(0x70), None);
    }
}
