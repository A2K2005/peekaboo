//! The empty window: Open, New from clipboard, a drop hint, and the recent
//! files list. The list is one Tab stop; arrow keys move between rows.
use super::{
    app::{add_tabs, display_path, file_name, invalidate, open, with_state, State},
    commands::Command,
    infobar,
    paint::Look,
    render::{fonts, measure, Align},
    widgets::{control_height, plain_widget, Layout, Rect, Region, Role, WidgetId},
};
use crate::integration;
use std::path::{Path, PathBuf};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
    UI::{
        Input::KeyboardAndMouse::{VIRTUAL_KEY, VK_DOWN, VK_END, VK_HOME, VK_UP},
        WindowsAndMessaging::{PostMessageW, WM_APP},
    },
};

/// Posted when the background check has rewritten the recent files list.
pub(super) const WM_APP_RECENT: u32 = WM_APP + 0x71;

/// Reads one small file and checks no paths, so it is safe at launch.
pub(super) fn load() -> Vec<PathBuf> {
    integration::recent_dir().map(|dir| integration::load_recent(&dir)).unwrap_or_default()
}

/// Drops files that no longer exist, on its own thread: a file on an
/// offline network share can take seconds to check.
pub(super) fn prune(hwnd: HWND) {
    let window = hwnd.0 as isize;
    let _ = std::thread::Builder::new().name("recent-files".into()).spawn(move || {
        if integration::recent_dir().is_some_and(|dir| integration::prune_recent(&dir).is_ok()) {
            unsafe {
                let _ = PostMessageW(Some(HWND(window as *mut _)), WM_APP_RECENT, WPARAM(0), LPARAM(0));
            }
        }
    });
}

pub(super) unsafe fn reload(hwnd: HWND) {
    with_state(|s| s.recent = load());
    invalidate(hwnd);
}

/// Untitled clipboard images stay in the temporary folder until the user
/// saves a copy, so they stay out of recent files.
fn untitled_dir() -> PathBuf {
    std::env::temp_dir().join("Peekaboo")
}

/// Records an opened file in the list and in Windows Recent. The shell call
/// can block, so it runs off the window thread.
pub(super) fn note(path: &Path) {
    if path.starts_with(untitled_dir()) {
        return;
    }
    let path = path.to_path_buf();
    let _ = std::thread::Builder::new().name("recent-note".into()).spawn(move || unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let _ = integration::note_recent(&path);
        if com {
            CoUninitialize();
        }
    });
}

/// Opens the clipboard image in a new tab as an untitled PNG.
pub(super) unsafe fn new_from_clipboard(hwnd: HWND) {
    let result = crate::imaging::clipboard_image().and_then(|frame| {
        let dir = untitled_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("Could not save the clipboard image. {e}"))?;
        let path = (1..10_000)
            .map(|n| dir.join(if n == 1 { "Untitled.png".to_string() } else { format!("Untitled {n}.png") }))
            .find(|path| !path.exists())
            .ok_or("Could not name the clipboard image. Close some untitled tabs and try again.")?;
        crate::imaging::export_frame(&frame, &path).map(|()| path)
    });
    match result {
        Ok(path) => {
            with_state(|s| {
                add_tabs(s, &[path.clone()]);
                s.focus = Some(WidgetId::Document);
            });
            open(hwnd, path);
        }
        Err(error) => {
            with_state(|s| infobar::error(s, error));
            invalidate(hwnd);
        }
    }
}

pub(super) unsafe fn open_recent(hwnd: HWND, index: usize) {
    let Some(path) = with_state(|s| s.recent.get(index).cloned()).flatten() else {
        return;
    };
    if path.exists() {
        with_state(|s| {
            add_tabs(s, &[path.clone()]);
            s.focus = Some(WidgetId::Document);
        });
        open(hwnd, path);
        return;
    }
    with_state(|s| {
        s.recent.retain(|p| *p != path);
        s.focus = (!s.recent.is_empty()).then(|| WidgetId::Recent(index.min(s.recent.len() - 1)));
        infobar::error(s, format!("{} was moved or deleted, so it is no longer in recent files.", file_name(&path)));
    });
    prune(hwnd);
    invalidate(hwnd);
}

/// Up, Down, Home, and End move through the rows.
pub(super) fn key(state: &mut State, vk: u16, index: usize) -> bool {
    let count = state.layout().widgets.iter().filter(|w| matches!(w.id, WidgetId::Recent(_))).count();
    if count == 0 {
        return false;
    }
    let next = match VIRTUAL_KEY(vk) {
        VK_UP => index.saturating_sub(1),
        VK_DOWN => index + 1,
        VK_HOME => 0,
        VK_END => count - 1,
        _ => return false,
    }
    .min(count - 1);
    state.focus = Some(WidgetId::Recent(next));
    state.focus_visible = true;
    true
}

/// Places the empty window's content, centered, with as many recent files
/// as fit.
pub(super) fn add(state: &State, layout: &mut Layout) {
    let Some(empty) = layout.empty.as_mut() else {
        return;
    };
    let Ok(fonts) = fonts(state.scale, state.text_scale) else {
        return;
    };
    let (s, ts, doc) = (state.scale, state.text_scale, layout.document);
    let control = control_height(ts) * s;
    let (title, line) = (28.0 * ts * s, 20.0 * ts * s);
    let fixed = title + 16.0 * s + control + 8.0 * s + line + 32.0 * s + line + 8.0 * s;
    let fit = ((doc.height() - 32.0 * s - fixed) / control).floor().max(1.0) as usize;
    let rows = fit.min(state.recent.len().max(1));
    let top = doc.y0 + ((doc.height() - fixed - rows as f32 * control) / 2.0).max(16.0 * s);
    let center = (doc.x0 + doc.x1) / 2.0;
    let text_width = (doc.width() - 32.0 * s).max(0.0);
    empty.heading = Rect::new(center - text_width / 2.0, top, text_width, title);

    let label = "New from clipboard";
    let clipboard_width = measure(label, &fonts.body, 10_000.0).0 + 32.0 * s;
    let open_width = 120f32.max(48.0 + 40.0 * ts) * s;
    let button_y = empty.heading.y1 + 16.0 * s;
    let left = center - (open_width + 8.0 * s + clipboard_width) / 2.0;
    if let Some(open) = layout.widgets.iter_mut().find(|w| w.id == WidgetId::Command(Command::Open) && w.region == Region::Document) {
        open.rect = Rect::new(left, button_y, open_width, control);
    }
    let clipboard_rect = Rect::new(left + open_width + 8.0 * s, button_y, clipboard_width, control);
    let mut clipboard = plain_widget(WidgetId::Command(Command::NewFromClipboard), Role::Button, Region::Document, clipboard_rect, label.into());
    clipboard.tooltip = "New from clipboard (Ctrl+N)".into();
    clipboard.focusable = state.sheet.is_none();
    layout.widgets.push(clipboard);

    empty.drop_hint = Rect::new(empty.heading.x0, button_y + control + 8.0 * s, text_width, line);
    empty.recent_heading = Rect::new(empty.heading.x0, empty.drop_hint.y1 + 32.0 * s, text_width, line);
    let list_width = text_width.min(520.0 * s);
    let list_x = center - list_width / 2.0;
    let list_y = empty.recent_heading.y1 + 8.0 * s;
    empty.recent = Rect::new(list_x, list_y, list_width, rows as f32 * control);
    let active = match state.focus {
        Some(WidgetId::Recent(index)) if index < rows => index,
        _ => 0,
    };
    for (index, path) in state.recent.iter().take(rows).enumerate() {
        let folder = path.parent().map(display_path).unwrap_or_default();
        let rect = Rect::new(list_x, list_y + index as f32 * control, list_width, control);
        let mut row = plain_widget(WidgetId::Recent(index), Role::ListItem, Region::Document, rect, format!("{}, {folder}", file_name(path)));
        row.tooltip = display_path(path);
        row.focusable = index == active && state.sheet.is_none();
        layout.widgets.push(row);
    }
}

/// Each row shows the file name, then its folder in secondary text.
pub(super) fn paint_recent(l: &Look, state: &State, layout: &Layout, list: Rect) {
    let s = l.s;
    let mut any = false;
    for w in layout.widgets.iter() {
        let WidgetId::Recent(index) = w.id else {
            continue;
        };
        let Some(path) = state.recent.get(index) else {
            continue;
        };
        any = true;
        if state.pressed == Some(w.id) {
            l.p.fill_round(w.rect, 4.0 * s, l.t.pressed);
        } else if state.hover == Some(w.id) {
            l.p.fill_round(w.rect, 4.0 * s, l.t.hover);
        }
        let inner = Rect { x0: w.rect.x0 + 12.0 * s, x1: w.rect.x1 - 12.0 * s, ..w.rect };
        let name = file_name(path);
        let name_width = (measure(&name, &l.f.body, inner.width()).0 + s).min(inner.width() * 0.6);
        l.p.text(&name, Rect { x1: inner.x0 + name_width, ..inner }, &l.f.body, l.t.text, Align::Leading);
        let folder = path.parent().map(display_path).unwrap_or_default();
        l.p.text(&folder, Rect { x0: inner.x0 + name_width + 16.0 * s, ..inner }, &l.f.caption, l.t.text_secondary, Align::Leading);
    }
    if !any {
        l.p.text("Files you open will show here.", list, &l.f.body, l.t.text_secondary, Align::Center);
    }
}
