//! The info bar and toasts, the one place for messages that need attention.
//! The bar holds errors, failed saves, outside changes, and offers until the
//! user acts. A toast reports a finished result and goes after a few
//! seconds. Both are UI Automation live regions, so Narrator reads them.
use super::{
    a11y, actions,
    app::{file_name, invalidate, with_state, SaveStatus, State, EMPTY_STATUS},
    commands::glyph,
    disk,
    paint::{icon_button, text_button, Look},
    render::{fonts, measure, Align},
    theme::{Mode, Rgba},
    widgets::{control_height, plain_widget, Layout, Rect, Region, Role, WidgetId},
};
use accesskit::{Live, Node, NodeId, Role as NodeRole};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
    UI::WindowsAndMessaging::{PostMessageW, WM_APP},
};

/// Posted by the default-app check when the offer should show.
pub(super) const WM_APP_OFFER: u32 = WM_APP + 0x70;
const TOAST_TIME: Duration = Duration::from_secs(4);
const OFFER_FILE: &str = "default-app-offer.txt";
const BAR_NODE: NodeId = NodeId(83_000);
const TOAST_NODE: NodeId = NodeId(83_001);
const INFO: u16 = 0xE946;
const WARNING: u16 = 0xE7BA;
const ERROR: u16 = 0xE783;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Offer,
    Error,
    Conflict,
}

#[derive(Clone)]
enum Action {
    OpenDefaultApps,
    NotNow,
    Resolve(PathBuf),
    Retry(PathBuf),
}

struct Bar {
    kind: Kind,
    text: String,
    actions: Vec<(&'static str, Action)>,
}

#[derive(Default)]
pub(super) struct Messages {
    /// The newest bar shows; closing it shows the one before.
    bars: Vec<Bar>,
    toast: Option<(String, Instant)>,
}

fn show(state: &mut State, bar: Bar) {
    // One error at a time; offers and conflicts repeat only for another file.
    state.messages.bars.retain(|b| b.kind != bar.kind || (bar.kind != Kind::Error && b.text != bar.text));
    state.messages.bars.push(bar);
}

pub(super) fn toast(state: &mut State, text: impl Into<String>) {
    state.messages.toast = Some((text.into(), Instant::now() + TOAST_TIME));
}

pub(super) fn error(state: &mut State, text: impl Into<String>) {
    show(state, Bar { kind: Kind::Error, text: text.into(), actions: Vec::new() });
}

/// A finished job: success as a toast, failure in the bar. The status line
/// goes back to the page, so it stops showing the job as running.
pub(super) fn report(state: &mut State, result: Result<String, String>) {
    state.status = if state.path.is_some() { state.subtitle() } else { EMPTY_STATUS.into() };
    match result {
        Ok(text) => toast(state, text),
        Err(text) => error(state, text),
    }
}

/// Shows the bar for an autosave problem with `path`, or clears it once the
/// file saves again.
pub(super) fn save_problem(state: &mut State, path: &Path) {
    let name = file_name(path);
    let owned = path.to_path_buf();
    match state.saves.get(path).map(|save| save.status.clone()) {
        Some(SaveStatus::Failed(error)) => show(
            state,
            Bar { kind: Kind::Error, text: format!("Could not save {name}. {error}"), actions: vec![("Retry", Action::Retry(owned))] },
        ),
        Some(SaveStatus::Conflict) => show(
            state,
            Bar {
                kind: Kind::Conflict,
                text: format!("{name} changed outside Preview. Autosave is paused."),
                actions: vec![("Resolve", Action::Resolve(owned))],
            },
        ),
        _ => state
            .messages
            .bars
            .retain(|bar| !bar.actions.iter().any(|(_, action)| matches!(action, Action::Retry(p) | Action::Resolve(p) if p == path))),
    }
}

/// Returns true when the toast just expired and the window needs a repaint.
pub(super) fn tick(state: &mut State) -> bool {
    let expired = state.messages.toast.as_ref().is_some_and(|(_, until)| Instant::now() >= *until);
    if expired {
        state.messages.toast = None;
    }
    expired
}

struct Geometry {
    card: Rect,
    icon: Rect,
    text: Rect,
    buttons: Vec<Rect>,
    close: Rect,
}

fn geometry(state: &State, doc: Rect) -> Option<Geometry> {
    let bar = state.messages.bars.last()?;
    let fonts = fonts(state.scale, state.text_scale).ok()?;
    let s = state.scale;
    let (pad, gap, control) = (8.0 * s, 8.0 * s, control_height(state.text_scale) * s);
    let (x0, x1) = (doc.x0 + pad, doc.x1 - pad);
    if x1 - x0 < 240.0 * s || doc.height() < 3.0 * control {
        return None;
    }
    let widths: Vec<f32> = bar.actions.iter().map(|(label, _)| measure(label, &fonts.body, 10_000.0).0 + 24.0 * s).collect();
    let buttons_width: f32 = widths.iter().map(|w| w + gap).sum();
    let icon_x = x0 + 12.0 * s;
    let text_x = icon_x + 20.0 * s + gap;
    let close_x = x1 - pad - control;
    // Buttons share the message's row while the message keeps 200 epx.
    let one_row = close_x - buttons_width - text_x >= 200.0 * s;
    let text_right = if one_row { close_x - buttons_width } else { close_x - gap };
    let text_height = measure(&bar.text, &fonts.wrap, (text_right - text_x).max(1.0)).1;
    let row = text_height.max(control);
    let top = doc.y0 + 2.0 * pad;
    let (mut x, y) = if one_row { (close_x - buttons_width, top + (row - control) / 2.0) } else { (text_x, top + row + gap) };
    let mut buttons = Vec::new();
    for width in widths {
        buttons.push(Rect::new(x, y, width, control));
        x += width + gap;
    }
    let rows = if one_row || buttons.is_empty() { row } else { row + gap + control };
    Some(Geometry {
        card: Rect { x0, y0: doc.y0 + pad, x1, y1: top + rows + pad },
        icon: Rect::new(icon_x, top + (row - control) / 2.0, 20.0 * s, control),
        text: Rect { x0: text_x, y0: top + (row - text_height) / 2.0, x1: text_right, y1: top + (row + text_height) / 2.0 },
        close: Rect::new(close_x, top + (row - control) / 2.0, control, control),
        buttons,
    })
}

/// Where content below the bar may start, so the find bar never covers it.
pub(super) fn bottom(state: &State, doc: Rect) -> f32 {
    geometry(state, doc).map_or(doc.y0, |g| g.card.y1)
}

fn toast_rect(state: &State, doc: Rect) -> Option<(&str, Rect)> {
    let (text, _) = state.messages.toast.as_ref()?;
    let fonts = fonts(state.scale, state.text_scale).ok()?;
    let s = state.scale;
    let height = control_height(state.text_scale) * s;
    let width = (measure(text, &fonts.body, 10_000.0).0 + 32.0 * s).min(doc.width() - 32.0 * s);
    (width > 0.0 && doc.height() > height + 32.0 * s)
        .then(|| (text.as_str(), Rect::new((doc.x0 + doc.x1 - width) / 2.0, doc.y1 - 16.0 * s - height, width, height)))
}

pub(super) fn add(state: &State, layout: &mut Layout) {
    let (Some(g), Some(bar)) = (geometry(state, layout.document), state.messages.bars.last()) else {
        return;
    };
    let modal = state.sheet.is_some();
    for (index, ((label, _), rect)) in bar.actions.iter().zip(g.buttons).enumerate() {
        let mut button = plain_widget(WidgetId::InfoButton(index), Role::Button, Region::InfoBar, rect, (*label).into());
        button.primary = index == 0;
        button.focusable = !modal;
        layout.widgets.push(button);
    }
    let mut close = plain_widget(WidgetId::InfoClose, Role::Button, Region::InfoBar, g.close, "Close message".into());
    close.glyph = Some(glyph::CANCEL);
    close.tooltip = "Close message".into();
    close.focusable = !modal;
    layout.widgets.push(close);
}

/// An opaque card with a soft edge, like the sheets.
pub(super) fn card(l: &Look, r: Rect) {
    let s = l.s;
    if l.t.mode != Mode::Contrast {
        for (spread, alpha) in [(6.0, 0.05), (2.0, 0.12)] {
            l.p.fill_round(r.inset(-spread * s), (8.0 + spread) * s, Rgba(0.0, 0.0, 0.0, alpha));
        }
    }
    l.p.fill_round(r, 8.0 * s, l.t.surface);
    l.p.stroke_round(r, 8.0 * s, l.t.control_border, l.t.border_width * s.floor().max(1.0));
}

pub(super) fn paint(l: &Look, state: &State, layout: &Layout) {
    if let (Some(g), Some(bar)) = (geometry(state, layout.document), state.messages.bars.last()) {
        card(l, g.card);
        let (icon, color) = match bar.kind {
            Kind::Error => (ERROR, l.t.close_hover),
            Kind::Conflict => (WARNING, l.t.accent),
            Kind::Offer => (INFO, l.t.accent),
        };
        l.p.glyph(icon, g.icon, &l.f.icon, color);
        l.p.text(&bar.text, g.text, &l.f.wrap, l.t.text, Align::Leading);
        for w in layout.widgets.iter().filter(|w| w.region == Region::InfoBar) {
            if w.glyph.is_some() {
                icon_button(l, state, w);
            } else {
                text_button(l, state, w);
            }
        }
    }
    if let Some((text, rect)) = toast_rect(state, layout.document) {
        card(l, rect);
        l.p.text(text, rect, &l.f.body, l.t.text, Align::Center);
    }
}

pub(super) fn a11y(state: &State, layout: &Layout, nodes: &mut Vec<(NodeId, Node)>, children: &mut Vec<NodeId>) {
    if let (Some(g), Some(bar)) = (geometry(state, layout.document), state.messages.bars.last()) {
        let urgent = bar.kind != Kind::Offer;
        let mut node = Node::new(if urgent { NodeRole::Alert } else { NodeRole::Group });
        node.set_label(bar.text.clone());
        node.set_bounds(a11y::bounds(g.card));
        node.set_live(if urgent { Live::Assertive } else { Live::Polite });
        node.set_children(
            layout.widgets.iter().filter(|w| w.region == Region::InfoBar).map(|w| a11y::node_id(w.id)).collect::<Vec<_>>(),
        );
        nodes.push((BAR_NODE, node));
        children.push(BAR_NODE);
    }
    if let Some((text, rect)) = toast_rect(state, layout.document) {
        let mut node = Node::new(NodeRole::Status);
        node.set_label(text);
        node.set_bounds(a11y::bounds(rect));
        node.set_live(Live::Polite);
        nodes.push((TOAST_NODE, node));
        children.push(TOAST_NODE);
    }
}

/// A click, Enter, or UI Automation Invoke on a bar button. Every button
/// closes the bar.
pub(super) unsafe fn activate(hwnd: HWND, id: WidgetId) {
    let action = with_state(|s| {
        let bar = s.messages.bars.pop()?;
        if matches!(s.focus, Some(WidgetId::InfoButton(_) | WidgetId::InfoClose)) {
            s.focus = s.path.as_ref().map(|_| WidgetId::Document);
        }
        match id {
            WidgetId::InfoButton(index) => bar.actions.get(index).map(|(_, action)| action.clone()),
            _ if bar.kind == Kind::Offer => Some(Action::NotNow),
            _ => None,
        }
    })
    .flatten();
    match action {
        Some(Action::OpenDefaultApps) => {
            let result = open_default_apps();
            remember_offer();
            if let Err(message) = result {
                with_state(|s| error(s, message));
            }
        }
        Some(Action::NotNow) => remember_offer(),
        Some(Action::Resolve(path)) => {
            actions::resolve_save_conflict(hwnd, path.clone());
            with_state(|s| save_problem(s, &path));
        }
        Some(Action::Retry(path)) => {
            with_state(|s| {
                if let Some(save) = s.saves.get_mut(&path).filter(|save| matches!(save.status, SaveStatus::Failed(_))) {
                    save.status = SaveStatus::Edited;
                    save.due = Some(Instant::now());
                }
            });
        }
        None => {}
    }
    invalidate(hwnd);
}

/// Default apps lists only registered apps, so a portable copy registers
/// first. Registration never changes a default and keeps Explorer's
/// thumbnail and preview handlers.
fn open_default_apps() -> Result<(), String> {
    if !crate::integration::is_registered() {
        let exe = std::env::current_exe().map_err(|e| format!("Could not find the Preview program file. {e}"))?;
        crate::integration::register(&exe)?;
    }
    crate::integration::open_default_apps()
}

fn remember_offer() {
    if let Some(dir) = disk::app_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(OFFER_FILE), "done");
    }
}

/// Offers once, on a launch after the first, to make Preview the default
/// viewer. Runs on its own thread after first content, so launch pays
/// nothing, and waits so the bar does not appear with the first page.
pub(super) fn after_launch(hwnd: HWND) {
    let window = hwnd.0 as isize;
    let _ = std::thread::Builder::new().name("default-app-check".into()).spawn(move || {
        std::thread::sleep(Duration::from_secs(2));
        let Some(dir) = disk::app_dir() else {
            return;
        };
        let file = dir.join(OFFER_FILE);
        match std::fs::read_to_string(&file) {
            Ok(text) if text.trim() == "done" => {}
            Ok(_) => unsafe {
                let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
                let default = [".pdf", ".jpg", ".png"].iter().all(|extension| crate::integration::is_default_for(extension));
                if com {
                    CoUninitialize();
                }
                if !default {
                    let _ = PostMessageW(Some(HWND(window as *mut _)), WM_APP_OFFER, WPARAM(0), LPARAM(0));
                }
            },
            Err(_) => {
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::write(&file, "seen");
            }
        }
    });
}

pub(super) unsafe fn offer(hwnd: HWND) {
    with_state(|s| {
        show(
            s,
            Bar {
                kind: Kind::Offer,
                text: "Make Preview for Windows your default viewer?".into(),
                actions: vec![("Open settings", Action::OpenDefaultApps), ("Not now", Action::NotNow)],
            },
        )
    });
    invalidate(hwnd);
}
