//! Custom-drawn popup menus: themed WS_POPUP windows with keyboard
//! navigation, access keys, submenus, and UI Automation. Win32 menus have no
//! documented dark mode (CLAUDE.md D11), so these replace HMENU.
//!
//! `track` works like TrackPopupMenu with TPM_RETURNCMD: it runs a nested
//! message loop and returns the chosen command.
use super::{
    commands::{access_key, glyph, MenuItem, Pick},
    render::{fonts, measure, Align, Painter},
    theme::Theme,
    widgets::{control_height, Rect},
};
use accesskit::{Action, ActionHandler, ActionRequest, ActivationHandler, Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate};
use std::cell::RefCell;
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::{Common::*, *},
            Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND},
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            Gdi::*,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
};

const WM_APP_INVOKE: u32 = WM_APP + 20;
const WM_APP_SELECT: u32 = WM_APP + 21;

struct Level {
    hwnd: HWND,
    items: Vec<MenuItem>,
    selected: Option<usize>,
    /// Row rectangles in client pixels.
    rows: Vec<Rect>,
    /// Item of this level whose submenu is open.
    open_child: Option<usize>,
}

struct Tracker {
    owner: HWND,
    levels: Vec<Level>,
    done: Option<Option<Pick>>,
    theme: Theme,
    scale: f32,
    text_scale: f32,
    painter: Option<(ID2D1DCRenderTarget, Painter)>,
}

thread_local! {
    static MENU: RefCell<Option<Tracker>> = const { RefCell::new(None) };
    static ADAPTERS: RefCell<Vec<(isize, accesskit_windows::Adapter)>> = const { RefCell::new(Vec::new()) };
}

fn label_text(label: &str) -> (String, Option<usize>) {
    let mut text = String::new();
    let mut underline = None;
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' && chars.peek().is_some() {
            underline = Some(text.encode_utf16().count());
            continue;
        }
        text.push(c);
    }
    (text, underline)
}

fn selectable(item: &MenuItem) -> bool {
    !item.is_separator()
}

/// Next selectable row after `from` in direction `step`, wrapping.
pub(super) fn step_selection(items: &[MenuItem], from: Option<usize>, forward: bool) -> Option<usize> {
    let n = items.len();
    if n == 0 {
        return None;
    }
    let mut index = match from {
        Some(i) => i,
        None if forward => n - 1,
        None => 0,
    };
    for _ in 0..n {
        index = if forward { (index + 1) % n } else { (index + n - 1) % n };
        if selectable(&items[index]) {
            return Some(index);
        }
    }
    None
}

/// What an access key does: activate a unique match, or move to the next of
/// several matches, as Win32 menus do.
#[derive(Debug, PartialEq)]
pub(super) enum KeyResult {
    Activate(usize),
    Select(usize),
    None,
}
pub(super) fn access(items: &[MenuItem], selected: Option<usize>, key: char) -> KeyResult {
    let key = key.to_ascii_uppercase();
    let matches: Vec<usize> =
        (0..items.len()).filter(|i| items[*i].enabled && access_key(&items[*i].label) == Some(key)).collect();
    match matches.len() {
        0 => KeyResult::None,
        1 => KeyResult::Activate(matches[0]),
        _ => KeyResult::Select(*matches.iter().find(|i| Some(**i) > selected).unwrap_or(&matches[0])),
    }
}

fn with<R>(f: impl FnOnce(&mut Tracker) -> R) -> Option<R> {
    MENU.with(|cell| cell.try_borrow_mut().ok().and_then(|mut t| t.as_mut().map(f)))
}

fn level_of(t: &Tracker, hwnd: HWND) -> Option<usize> {
    t.levels.iter().position(|l| l.hwnd == hwnd)
}

/// Size in pixels and row rectangles for a list of items.
fn measure_items(items: &[MenuItem], scale: f32, text_scale: f32) -> (f32, f32, Vec<Rect>) {
    let fonts = fonts(scale, text_scale).ok();
    let row = control_height(text_scale) * scale;
    let mut width: f32 = 200.0 * scale;
    let mut y = 4.0 * scale;
    let mut rows = Vec::new();
    for item in items {
        let h = if item.is_separator() { 9.0 * scale } else { row };
        rows.push(Rect::new(0.0, y, 0.0, h));
        y += h;
        if let Some(fonts) = &fonts {
            let label = measure(&label_text(&item.label).0, &fonts.body, 10_000.0).0;
            let keys = measure(&item.shortcut, &fonts.body, 10_000.0).0;
            width = width.max(36.0 * scale + label + 32.0 * scale + keys + 32.0 * scale);
        }
    }
    let width = width.min(480.0 * scale).ceil();
    for r in &mut rows {
        r.x0 = 0.0;
        r.x1 = width;
    }
    (width, (y + 4.0 * scale).ceil(), rows)
}

/// Keeps a popup inside the monitor work area. Submenus flip to the left of
/// their parent when they do not fit on the right.
pub(super) fn place(anchor: (f32, f32), flip_x: f32, size: (f32, f32), work: Rect) -> (f32, f32) {
    let mut x = anchor.0;
    if x + size.0 > work.x1 {
        x = (flip_x - size.0).max(work.x0);
    }
    let mut y = anchor.1;
    if y + size.1 > work.y1 {
        y = (work.y1 - size.1).max(work.y0);
    }
    (x.max(work.x0), y.max(work.y0))
}

unsafe fn register() -> PCWSTR {
    let class = w!("PreviewMenu");
    let wc = WNDCLASSW {
        // CS_DROPSHADOW gives the standard popup shadow.
        style: CS_DROPSHADOW,
        lpfnWndProc: Some(menu_proc),
        hInstance: GetModuleHandleW(None).map(Into::into).unwrap_or_default(),
        lpszClassName: class,
        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
        ..Default::default()
    };
    RegisterClassW(&wc);
    class
}

/// Opens a popup for `items` with its top-left at `anchor` (screen pixels).
/// `flip_x` is where the right edge goes when the popup flips left.
unsafe fn open_level(items: Vec<MenuItem>, anchor: (f32, f32), flip_x: f32, select_first: bool) {
    let Some((owner, scale, text_scale, theme)) =
        with(|t| (t.levels.last().map_or(t.owner, |l| l.hwnd), t.scale, t.text_scale, t.theme))
    else {
        return;
    };
    let (width, height, rows) = measure_items(&items, scale, text_scale);
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    let monitor = MonitorFromPoint(POINT { x: anchor.0 as i32, y: anchor.1 as i32 }, MONITOR_DEFAULTTONEAREST);
    let _ = GetMonitorInfoW(monitor, &mut info);
    let work = Rect {
        x0: info.rcWork.left as f32,
        y0: info.rcWork.top as f32,
        x1: info.rcWork.right as f32,
        y1: info.rcWork.bottom as f32,
    };
    let (x, y) = place(anchor, flip_x, (width, height), work);
    let class = register();
    let Ok(hwnd) = CreateWindowExW(
        WS_EX_TOOLWINDOW,
        class,
        w!("Menu"),
        WS_POPUP,
        x as i32,
        y as i32,
        width as i32,
        height as i32,
        Some(owner),
        None,
        GetModuleHandleW(None).ok().map(Into::into),
        None,
    ) else {
        return;
    };
    // Windows 11 rounds popups; Windows 10 ignores the attribute.
    // https://learn.microsoft.com/windows/apps/desktop/modernize/ui/apply-rounded-corners
    let corner = DWMWCP_ROUND;
    let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &corner as *const _ as _, 4);
    let selected = if select_first { step_selection(&items, None, true) } else { None };
    with(|t| t.levels.push(Level { hwnd, items, selected, rows, open_child: None }));
    let adapter = accesskit_windows::Adapter::new(hwnd, true, Invoker(hwnd.0 as isize));
    ADAPTERS.with(|a| a.borrow_mut().push((hwnd.0 as isize, adapter)));
    let _ = theme;
    // The popup takes activation so keyboard input and UI Automation focus
    // follow the selected item.
    let _ = ShowWindow(hwnd, SW_SHOW);
}

/// Destroys every level deeper than `keep`.
unsafe fn close_after(keep: usize) {
    let doomed: Vec<HWND> = with(|t| {
        if let Some(level) = t.levels.get_mut(keep) {
            level.open_child = None;
        }
        t.levels.drain(keep + 1..).map(|l| l.hwnd).collect()
    })
    .unwrap_or_default();
    for hwnd in doomed.into_iter().rev() {
        ADAPTERS.with(|a| a.borrow_mut().retain(|(h, _)| *h != hwnd.0 as isize));
        let _ = DestroyWindow(hwnd);
    }
}

fn finish(result: Option<Pick>) {
    with(|t| {
        if t.done.is_none() {
            t.done = Some(result);
        }
    });
}

unsafe fn invalidate(hwnd: HWND) {
    let _ = InvalidateRect(Some(hwnd), None, false);
}

/// Activates an item: a command closes the menu; a submenu opens.
unsafe fn activate(level: usize, index: usize) {
    let Some(action) = with(|t| {
        let l = t.levels.get_mut(level)?;
        let item = l.items.get(index)?;
        if item.is_separator() || !item.enabled {
            return None;
        }
        l.selected = Some(index);
        if !item.children.is_empty() {
            let mut point = POINT { x: l.rows[index].x1 as i32, y: l.rows[index].y0 as i32 };
            let mut left = POINT { x: 0, y: 0 };
            let _ = ClientToScreen(l.hwnd, &mut point);
            let _ = ClientToScreen(l.hwnd, &mut left);
            let open = l.open_child == Some(index);
            l.open_child = Some(index);
            Some(Err((item.children.clone(), (point.x as f32 - 4.0, point.y as f32 - 4.0), left.x as f32 + 4.0, open)))
        } else {
            Some(Ok(item.pick))
        }
    })
    .flatten() else {
        return;
    };
    match action {
        Ok(command) => finish(command),
        Err((children, anchor, flip_x, already_open)) => {
            if already_open {
                // Move into the open submenu.
                let child = with(|t| t.levels.get(level + 1).map(|l| l.hwnd)).flatten();
                if let Some(child) = child {
                    let _ = SetActiveWindow(child);
                    select(level + 1, None, true);
                }
                return;
            }
            close_after(level);
            with(|t| t.levels[level].open_child = Some(index));
            open_level(children, anchor, flip_x, true);
        }
    }
}

/// Selects `index`, or steps from the current selection when `index` is None.
unsafe fn select(level: usize, index: Option<usize>, forward: bool) {
    let hwnd = with(|t| {
        let l = t.levels.get_mut(level)?;
        l.selected = index.or_else(|| step_selection(&l.items, l.selected, forward));
        Some(l.hwnd)
    })
    .flatten();
    if let Some(hwnd) = hwnd {
        invalidate(hwnd);
        update_a11y(hwnd);
    }
}

pub(super) unsafe fn track(
    owner: HWND,
    items: Vec<MenuItem>,
    anchor: (f32, f32),
    keyboard: bool,
    theme: Theme,
    scale: f32,
    text_scale: f32,
) -> Option<Pick> {
    if items.is_empty() || MENU.with(|m| m.borrow().is_some()) {
        return None;
    }
    MENU.with(|m| {
        *m.borrow_mut() =
            Some(Tracker { owner, levels: Vec::new(), done: None, theme, scale, text_scale, painter: None })
    });
    open_level(items, anchor, anchor.0, keyboard);
    let mut message = MSG::default();
    while with(|t| t.done.is_none() && !t.levels.is_empty()).unwrap_or(false) {
        let result = GetMessageW(&mut message, None, 0, 0).0;
        if result <= 0 {
            // WM_QUIT arrived inside the nested loop; pass it on to the main loop.
            PostQuitMessage(message.wParam.0 as i32);
            break;
        }
        let _ = TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    close_after(0);
    let tracker = MENU.with(|m| m.borrow_mut().take());
    if let Some(root) = tracker.as_ref().and_then(|t| t.levels.first()) {
        ADAPTERS.with(|a| a.borrow_mut().clear());
        let _ = DestroyWindow(root.hwnd);
    }
    tracker.and_then(|t| t.done.flatten())
}

unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let _ = with(|t| -> Result<()> {
        let level = level_of(t, hwnd).ok_or(Error::from_hresult(E_FAIL))?;
        if t.painter.is_none() {
            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            // A software target avoids creating a GPU device for a small popup.
            let target = factory.CreateDCRenderTarget(&D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_IGNORE },
                dpiX: 96.0,
                dpiY: 96.0,
                ..Default::default()
            })?;
            let painter = Painter::new(target.cast()?)?;
            t.painter = Some((target, painter));
        }
        let (target, painter) = t.painter.as_ref().unwrap();
        let mut client = RECT::default();
        GetClientRect(hwnd, &mut client)?;
        target.BindDC(hdc, &client)?;
        let theme = &t.theme;
        let s = t.scale;
        let fonts = fonts(s, t.text_scale)?;
        let l = &t.levels[level];
        painter.target.BeginDraw();
        painter.clear(theme.surface);
        let bounds = Rect { x0: 0.0, y0: 0.0, x1: client.right as f32, y1: client.bottom as f32 };
        painter.stroke_round(bounds, 0.0, theme.border, theme.border_width * s.max(1.0).floor());
        for (index, (item, row)) in l.items.iter().zip(&l.rows).enumerate() {
            if item.is_separator() {
                let y = (row.y0 + row.y1) / 2.0;
                painter.line((row.x0 + 4.0 * s, y), (row.x1 - 4.0 * s, y), theme.border, s.max(1.0).floor());
                continue;
            }
            let selected = l.selected == Some(index);
            let color = if !item.enabled {
                theme.text_disabled
            } else if selected {
                theme.hover_text
            } else {
                theme.text
            };
            if selected {
                painter.fill_round(Rect { x0: row.x0 + 4.0 * s, y0: row.y0 + 2.0 * s, x1: row.x1 - 4.0 * s, y1: row.y1 - 2.0 * s }, 4.0 * s, theme.hover);
            }
            if item.checked == Some(true) {
                painter.glyph(glyph::CHECK, Rect { x0: row.x0 + 8.0 * s, y0: row.y0, x1: row.x0 + 32.0 * s, y1: row.y1 }, &fonts.icon, color);
            }
            let (text, underline) = label_text(&item.label);
            let label = Rect { x0: row.x0 + 36.0 * s, y0: row.y0, x1: row.x1 - 28.0 * s, y1: row.y1 };
            let (_, text_h) = measure(&text, &fonts.body, label.width());
            let label = Rect { y0: row.y0 + (row.height() - text_h) / 2.0, ..label };
            painter.text_underlined(&text, underline, label, &fonts.body, color);
            if !item.shortcut.is_empty() {
                let keys = Rect { x0: row.x0 + 36.0 * s, y0: row.y0, x1: row.x1 - 12.0 * s, y1: row.y1 };
                let secondary = if selected { color } else if item.enabled { theme.text_secondary } else { theme.text_disabled };
                painter.text(&item.shortcut, keys, &fonts.body, secondary, Align::Trailing);
            }
            if !item.children.is_empty() {
                let chevron = Rect { x0: row.x1 - 28.0 * s, y0: row.y0, x1: row.x1 - 8.0 * s, y1: row.y1 };
                painter.glyph(glyph::CHEVRON_RIGHT, chevron, &fonts.caption_icon, color);
            }
        }
        painter.target.EndDraw(None, None)
    });
    let _ = EndPaint(hwnd, &ps);
}

fn item_at(t: &Tracker, level: usize, y: f32) -> Option<usize> {
    t.levels.get(level)?.rows.iter().position(|r| y >= r.y0 && y < r.y1)
}

unsafe extern "system" fn menu_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let level = with(|t| level_of(t, hwnd)).flatten();
    match (message, level) {
        (WM_PAINT, _) => {
            paint(hwnd);
            LRESULT(0)
        }
        (WM_ERASEBKGND, _) => LRESULT(1),
        (WM_MOUSEACTIVATE, _) => LRESULT(MA_ACTIVATE as isize),
        (WM_MOUSEMOVE, Some(level)) => {
            let y = ((lparam.0 >> 16) as u16 as i16) as f32;
            let hovered = with(|t| item_at(t, level, y)).flatten();
            let changed = with(|t| t.levels[level].selected != hovered && hovered.is_some()).unwrap_or(false);
            if let (Some(index), true) = (hovered, changed) {
                select(level, Some(index), true);
                let submenu = with(|t| {
                    let item = &t.levels[level].items[index];
                    item.enabled && !item.children.is_empty()
                })
                .unwrap_or(false);
                if submenu {
                    activate(level, index);
                } else {
                    close_after(level);
                }
            }
            LRESULT(0)
        }
        (WM_LBUTTONUP, Some(level)) => {
            let y = ((lparam.0 >> 16) as u16 as i16) as f32;
            if let Some(index) = with(|t| item_at(t, level, y)).flatten() {
                activate(level, index);
            }
            LRESULT(0)
        }
        (WM_KEYDOWN, Some(level)) => {
            let selected = with(|t| t.levels[level].selected).flatten();
            match VIRTUAL_KEY(wparam.0 as u16) {
                VK_DOWN => select(level, None, true),
                VK_UP => select(level, None, false),
                VK_HOME => {
                    with(|t| t.levels[level].selected = None);
                    select(level, None, true)
                }
                VK_END => {
                    with(|t| t.levels[level].selected = None);
                    select(level, None, false)
                }
                VK_RETURN | VK_SPACE => {
                    if let Some(index) = selected {
                        activate(level, index);
                    }
                }
                VK_RIGHT => {
                    let submenu = selected.is_some_and(|i| with(|t| !t.levels[level].items[i].children.is_empty()).unwrap_or(false));
                    if submenu {
                        activate(level, selected.unwrap());
                    }
                }
                VK_LEFT | VK_ESCAPE if level > 0 => close_after(level - 1),
                VK_ESCAPE => finish(None),
                _ => {}
            }
            LRESULT(0)
        }
        (WM_CHAR, Some(level)) => {
            if let Some(key) = char::from_u32(wparam.0 as u32) {
                let result = with(|t| access(&t.levels[level].items, t.levels[level].selected, key));
                match result {
                    Some(KeyResult::Activate(index)) => activate(level, index),
                    Some(KeyResult::Select(index)) => select(level, Some(index), true),
                    _ => {}
                }
            }
            LRESULT(0)
        }
        // Alt or F10 closes menus, as in Win32 menus.
        (WM_SYSKEYDOWN, _) => {
            finish(None);
            LRESULT(0)
        }
        (WM_ACTIVATE, _) => {
            if (wparam.0 & 0xffff) as u32 == WA_INACTIVE {
                let other = HWND(lparam.0 as *mut _);
                let ours = with(|t| level_of(t, other).is_some()).unwrap_or(false);
                if !ours {
                    finish(None);
                }
            }
            LRESULT(0)
        }
        (WM_SETFOCUS | WM_KILLFOCUS, _) => {
            let focused = message == WM_SETFOCUS;
            let events = ADAPTERS.with(|a| {
                let mut adapters = a.try_borrow_mut().ok()?;
                let (_, adapter) = adapters.iter_mut().find(|(h, _)| *h == hwnd.0 as isize)?;
                adapter.update_window_focus_state(focused)
            });
            if let Some(events) = events {
                events.raise();
            }
            LRESULT(0)
        }
        (WM_APP_INVOKE, Some(level)) => {
            activate(level, wparam.0);
            LRESULT(0)
        }
        (WM_APP_SELECT, Some(level)) => {
            select(level, Some(wparam.0), true);
            LRESULT(0)
        }
        (WM_GETOBJECT, _) => {
            let result = ADAPTERS.with(|a| {
                let mut adapters = a.try_borrow_mut().ok()?;
                let (_, adapter) = adapters.iter_mut().find(|(h, _)| *h == hwnd.0 as isize)?;
                adapter.handle_wm_getobject(wparam, lparam, &mut MenuTree(hwnd.0 as isize))
            });
            match result {
                Some(result) => result.into(),
                None => DefWindowProcW(hwnd, message, wparam, lparam),
            }
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

/// UI Automation: a Menu with MenuItem children; focus follows selection.
fn menu_tree(hwnd: isize) -> Option<TreeUpdate> {
    MENU.with(|cell| {
        let t = cell.try_borrow().ok()?;
        let t = t.as_ref()?;
        let l = t.levels.iter().find(|l| l.hwnd.0 as isize == hwnd)?;
        let root = NodeId(1);
        let mut nodes = Vec::new();
        let mut children = Vec::new();
        for (index, (item, row)) in l.items.iter().zip(&l.rows).enumerate() {
            if item.is_separator() {
                continue;
            }
            let id = NodeId(100 + index as u64);
            let mut node = Node::new(if item.checked.is_some() { Role::MenuItemCheckBox } else { Role::MenuItem });
            node.set_label(label_text(&item.label).0);
            node.set_bounds(accesskit::Rect { x0: row.x0 as f64, y0: row.y0 as f64, x1: row.x1 as f64, y1: row.y1 as f64 });
            if let Some(key) = access_key(&item.label) {
                node.set_access_key(key.to_string());
            }
            if !item.shortcut.is_empty() {
                node.set_keyboard_shortcut(item.shortcut.clone());
            }
            if let Some(checked) = item.checked {
                node.set_toggled(checked.into());
            }
            if !item.children.is_empty() {
                node.set_expanded(l.open_child == Some(index));
            }
            if item.enabled {
                node.add_action(Action::Click);
                node.add_action(Action::Focus);
            } else {
                node.set_disabled();
            }
            nodes.push((id, node));
            children.push(id);
        }
        let mut menu = Node::new(Role::Menu);
        menu.set_label("Menu");
        menu.set_children(children);
        nodes.push((root, menu));
        Some(TreeUpdate {
            nodes,
            tree: Some(TreeInfo::new(root)),
            tree_id: TreeId::ROOT,
            focus: l.selected.map_or(root, |i| NodeId(100 + i as u64)),
        })
    })
}

struct MenuTree(isize);
impl ActivationHandler for MenuTree {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        menu_tree(self.0)
    }
}

unsafe fn update_a11y(hwnd: HWND) {
    let Some(update) = menu_tree(hwnd.0 as isize) else {
        return;
    };
    let events = ADAPTERS.with(|a| {
        let mut adapters = a.try_borrow_mut().ok()?;
        let (_, adapter) = adapters.iter_mut().find(|(h, _)| *h == hwnd.0 as isize)?;
        adapter.update_if_active(|| update)
    });
    if let Some(events) = events {
        events.raise();
    }
}

/// Runs on a UI Automation thread, so it only posts to the menu window.
struct Invoker(isize);
impl ActionHandler for Invoker {
    fn do_action(&mut self, request: ActionRequest) {
        let Some(index) = request.target_node.0.checked_sub(100) else {
            return;
        };
        let message = match request.action {
            Action::Click => WM_APP_INVOKE,
            Action::Focus => WM_APP_SELECT,
            _ => return,
        };
        unsafe {
            let _ = PostMessageW(Some(HWND(self.0 as *mut _)), message, WPARAM(index as usize), LPARAM(0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(label: &str, enabled: bool) -> MenuItem {
        MenuItem { label: label.into(), enabled, ..MenuItem::choice("", 0) }
    }

    #[test]
    fn arrow_keys_skip_separators_and_wrap() {
        let items = vec![item("&Open", true), MenuItem::separator(), item("&Print", false), item("E&xit", true)];
        assert_eq!(step_selection(&items, None, true), Some(0));
        assert_eq!(step_selection(&items, Some(0), true), Some(2), "disabled items still get focus, as in Win32");
        assert_eq!(step_selection(&items, Some(3), true), Some(0));
        assert_eq!(step_selection(&items, Some(0), false), Some(3));
        assert_eq!(step_selection(&items, None, false), Some(3));
        assert_eq!(step_selection(&[MenuItem::separator()], None, true), None);
    }

    #[test]
    fn access_keys_activate_unique_items_and_cycle_duplicates() {
        let items = vec![item("&Open", true), item("&Print", false), item("&Other", true), item("&Zoom", true)];
        assert_eq!(access(&items, None, 'z'), KeyResult::Activate(3));
        assert_eq!(access(&items, None, 'p'), KeyResult::None, "disabled");
        assert_eq!(access(&items, None, 'o'), KeyResult::Select(0));
        assert_eq!(access(&items, Some(0), 'O'), KeyResult::Select(2));
        assert_eq!(access(&items, Some(2), 'O'), KeyResult::Select(0));
    }

    #[test]
    fn labels_drop_the_ampersand_and_mark_the_key() {
        assert_eq!(label_text("Save a &copy..."), ("Save a copy...".into(), Some(7)));
        assert_eq!(label_text("Plain"), ("Plain".into(), None));
    }

    #[test]
    fn popups_stay_on_screen_and_submenus_flip_left() {
        let work = Rect { x0: 0.0, y0: 0.0, x1: 1000.0, y1: 800.0 };
        assert_eq!(place((100.0, 100.0), 100.0, (200.0, 300.0), work), (100.0, 100.0));
        assert_eq!(place((900.0, 100.0), 880.0, (200.0, 300.0), work), (680.0, 100.0));
        assert_eq!(place((100.0, 700.0), 100.0, (200.0, 300.0), work), (100.0, 500.0));
        assert_eq!(place((-50.0, -50.0), 0.0, (200.0, 300.0), work), (0.0, 0.0));
    }
}
