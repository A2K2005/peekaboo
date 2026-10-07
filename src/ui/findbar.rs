//! The find bar: a text box that searches as you type, a match count,
//! previous and next, and match case. The text box is an EDIT child, so IME
//! and text services work (CLAUDE.md D11). PDFs search on the document
//! worker; images search their OCR text layer on the window thread.
use super::{
    a11y, actions,
    app::{invalidate, with_state, State},
    commands::{glyph, Command},
    document, infobar,
    paint::{icon_button, Look},
    render::Align,
    sheet,
    theme::Mode,
    widgets::{self, control_height, plain_widget, Layout, Rect, Region, Role, WidgetId},
    worker::{is_pdf, Job, Request},
};
use crate::model::SearchHit;
use accesskit::{Live, Node, NodeId, Role as NodeRole};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use windows::{
    core::w,
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        Graphics::Gdi::InvalidateRect,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::EM_SETSEL,
            Input::KeyboardAndMouse::*,
            WindowsAndMessaging::*,
        },
    },
};

/// Typing pauses this long before a search starts.
const DEBOUNCE: Duration = Duration::from_millis(150);
const CHEVRON_UP: u16 = 0xE70E;
const CHEVRON_DOWN: u16 = 0xE70D;
const BAR_NODE: NodeId = NodeId(83_002);
const COUNT_NODE: NodeId = NodeId(83_003);

/// One search. The worker stops between pages once `cancel` is set.
pub(super) struct Query {
    pub(super) text: String,
    pub(super) match_case: bool,
    pub(super) cancel: Arc<AtomicBool>,
}

impl From<&str> for Query {
    fn from(text: &str) -> Self {
        Query { text: text.into(), match_case: false, cancel: Arc::default() }
    }
}

/// Generation, file, text, and match case of a search.
type Key = (u64, PathBuf, String, bool);

#[derive(Default)]
pub(super) struct FindBar {
    open: bool,
    edit: Option<HWND>,
    text: String,
    match_case: bool,
    due: Option<Instant>,
    sent: Option<Key>,
    cancel: Option<Arc<AtomicBool>>,
    /// Text and match case of the hits on screen.
    applied: Option<(String, bool)>,
    /// Searches the document worker has not answered. It answers in order,
    /// so only the answer that brings this to 0 is current.
    in_flight: u32,
    /// EDIT position, font, and theme last applied.
    placed: Option<(i32, i32, i32, i32, isize, Mode)>,
}

fn key(state: &State) -> Option<Key> {
    Some((state.generation, state.path.clone()?, state.find.text.clone(), state.find.match_case))
}

fn searching(state: &State) -> bool {
    !state.find.text.is_empty() && (state.find.in_flight > 0 || state.find.sent != key(state))
}

fn count(state: &State) -> String {
    if state.find.text.is_empty() {
        String::new()
    } else if searching(state) {
        "Searching...".into()
    } else if state.find_hits.is_empty() {
        "No results".into()
    } else {
        format!("{} of {}", state.find_index.min(state.find_hits.len() - 1) + 1, state.find_hits.len())
    }
}

/// Starts the search the text box asks for, once typing pauses. Returns true
/// when the bar needs a repaint.
pub(super) fn tick(state: &mut State) -> bool {
    if !state.find.open || state.pending || state.render_failed || state.find.due.is_some_and(|due| Instant::now() < due) {
        return false;
    }
    let Some(key) = key(state) else {
        return false;
    };
    if state.find.sent.as_ref() == Some(&key) {
        return false;
    }
    state.find.due = None;
    if let Some(cancel) = state.find.cancel.take() {
        cancel.store(true, Ordering::Release);
    }
    if key.2.is_empty() {
        state.find.sent = Some(key);
        state.find.applied = None;
        state.find_hits.clear();
        state.find_query.clear();
        state.find_index = 0;
        return true;
    }
    if !is_pdf(&key.1) {
        if let Some(layer) = state.text_layers.get(&0) {
            let hits = super::text::find(layer, 0, &key.2, key.3);
            state.find.sent = Some(key);
            apply(state, hits);
            return true;
        }
        if let Some(error) = state.text_failed.get(&0).cloned() {
            state.find.sent = Some(key);
            state.find_hits.clear();
            infobar::error(state, error);
            return true;
        }
        // OCR runs once per image; later ticks search its layer.
        state.request_text_layer(0);
        return false;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let request = Request {
        generation: state.generation,
        path: key.1.clone(),
        page: state.page,
        delta: 0,
        width: 1,
        height: 1,
        sessions: state.sessions.clone(),
        sources: state.opened_sources(),
    };
    let query = Query { text: key.2.clone(), match_case: key.3, cancel: cancel.clone() };
    state.find.sent = Some(key);
    if state.send(Job::Find(request, query)) {
        state.find.cancel = Some(cancel);
        state.find.in_flight += 1;
    } else {
        infobar::error(state, "Search stopped working. Close Preview and open the file again.");
    }
    true
}

/// Shows `hits` from the current page on. Only new text moves the view: a
/// search repeated after an edit or a tab switch leaves the view in place.
fn apply(state: &mut State, hits: Vec<SearchHit>) {
    let query = (state.find.text.clone(), state.find.match_case);
    let fresh = state.find.applied.as_ref() != Some(&query);
    state.find.applied = Some(query);
    let index = hits.iter().position(|hit| hit.page >= state.page).unwrap_or(0);
    state.find_query = state.find.text.clone();
    state.find_hits = hits;
    state.find_index = index;
    if let Some(page) = state.find_hits.get(index).map(|hit| hit.page).filter(|_| fresh) {
        if state.pdf.is_some() {
            if page != state.page {
                document::go_to_page(state, page, true);
            }
        } else {
            state.page = page;
        }
    }
}

/// A search result from the document worker. Returns true when it is shown.
pub(super) fn found(state: &mut State, generation: u64, path: PathBuf, result: Result<Vec<SearchHit>, String>) -> bool {
    state.find.in_flight = state.find.in_flight.saturating_sub(1);
    let current = state.find.in_flight == 0
        && state.find.open
        && generation == state.generation
        && state.path.as_ref() == Some(&path)
        && state.find.sent == key(state);
    if !current {
        return false;
    }
    match result {
        Ok(hits) => apply(state, hits),
        // Typing canceled it, then went back to the same text: search again.
        Err(error) if error == crate::pdf::SEARCH_CANCELED => state.find.sent = None,
        Err(error) => {
            state.find_hits.clear();
            infobar::error(state, error);
        }
    }
    true
}

/// Ctrl+F: shows the bar and selects its text.
pub(super) unsafe fn open(hwnd: HWND) {
    let Some(edit) = with_state(|s| {
        s.find.open = true;
        s.find.edit
    }) else {
        return;
    };
    let edit = match edit {
        Some(edit) => edit,
        None => {
            let style = WS_CHILD | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32);
            let instance = GetModuleHandleW(None).ok().map(Into::into);
            let Ok(edit) =
                CreateWindowExW(WINDOW_EX_STYLE(0), w!("EDIT"), w!(""), style, 0, 0, 0, 0, Some(hwnd), Some(HMENU(2000 as *mut _)), instance, None)
            else {
                return;
            };
            sheet::name_field(edit, "Find in document");
            with_state(|s| s.find.edit = Some(edit));
            edit
        }
    };
    place(hwnd);
    // A window too narrow for the bar keeps the EDIT hidden and unfocused.
    if with_state(|s| s.find.placed.is_some()) == Some(true) {
        let _ = SetFocus(Some(edit));
        SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
    }
    invalidate(hwnd);
}

/// Esc or the close button: hides the bar, clears the highlights, and
/// returns focus to the document.
pub(super) unsafe fn close(hwnd: HWND) {
    with_state(|s| {
        s.find.open = false;
        if let Some(cancel) = s.find.cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        s.find.sent = None;
        s.find.due = None;
        s.find.applied = None;
        s.find_hits.clear();
        s.find_query.clear();
        s.find_index = 0;
        let in_bar = s.focus.is_none_or(|focus| {
            matches!(
                focus,
                WidgetId::FindField
                    | WidgetId::FindCase
                    | WidgetId::FindClose
                    | WidgetId::Command(Command::FindNext | Command::FindPrevious)
            )
        });
        if in_bar {
            s.focus = s.path.as_ref().map(|_| WidgetId::Document);
        }
    });
    place(hwnd);
    let _ = SetFocus(Some(hwnd));
    invalidate(hwnd);
}

/// Moves the EDIT onto the drawn text box, and hides it while the bar or a
/// sheet hides the box. Runs from the window's timer, so it follows every
/// layout change.
pub(super) unsafe fn place(hwnd: HWND) {
    let Some((edit, target)) = with_state(|s| {
        let edit = s.find.edit?;
        let field = if s.find.open && s.sheet.is_none() { geometry(s, s.layout().document).map(|g| g.field) } else { None };
        let target = field.map(|r| {
            let line = 20.0 * s.text_scale * s.scale;
            let pad = 10.0 * s.scale;
            let font = sheet::edit_font(s.scale, s.text_scale);
            (
                (r.x0 + pad) as i32,
                (r.y0 + (r.height() - line) / 2.0) as i32,
                (r.width() - 2.0 * pad).max(1.0) as i32,
                line.ceil() as i32,
                font.0 as isize,
                s.theme.mode,
            )
        });
        if s.find.placed == target {
            return None;
        }
        s.find.placed = target;
        Some((edit, target))
    })
    .flatten() else {
        return;
    };
    match target {
        Some((x, y, width, height, font, _)) => {
            SendMessageW(edit, WM_SETFONT, Some(WPARAM(font as usize)), Some(LPARAM(0)));
            let _ = MoveWindow(edit, x, y, width, height, true);
            let _ = ShowWindow(edit, SW_SHOWNA);
            // Repaints with the theme's colors from WM_CTLCOLOREDIT.
            let _ = InvalidateRect(Some(edit), None, true);
        }
        None => {
            if GetFocus() == edit {
                let _ = SetFocus(Some(hwnd));
            }
            let _ = ShowWindow(edit, SW_HIDE);
        }
    }
}

/// WM_COMMAND from the EDIT. Returns false for any other control.
pub(super) unsafe fn command(hwnd: HWND, code: u32, control: HWND) -> bool {
    if control.is_invalid() || with_state(|s| s.find.edit) != Some(Some(control)) {
        return false;
    }
    match code {
        EN_CHANGE => {
            let mut text = vec![0u16; GetWindowTextLengthW(control).max(0) as usize + 1];
            let length = GetWindowTextW(control, &mut text).max(0) as usize;
            let text = String::from_utf16_lossy(&text[..length]);
            with_state(|s| {
                s.find.text = text;
                s.find.due = Some(Instant::now() + DEBOUNCE);
                if let Some(cancel) = s.find.cancel.take() {
                    cancel.store(true, Ordering::Release);
                }
            });
        }
        EN_SETFOCUS => {
            with_state(|s| s.focus = Some(WidgetId::FindField));
        }
        EN_KILLFOCUS => {
            with_state(|s| {
                if s.focus == Some(WidgetId::FindField) {
                    s.focus = s.path.as_ref().map(|_| WidgetId::Document);
                }
            });
        }
        _ => {}
    }
    invalidate(hwnd);
    true
}

/// Keys the EDIT would otherwise swallow or beep at. Runs in the message
/// loop before dispatch. Returns true when the key was handled.
pub(super) unsafe fn pre_dispatch(hwnd: HWND, message: &MSG) -> bool {
    if message.message != WM_KEYDOWN || with_state(|s| s.find.edit) != Some(Some(message.hwnd)) {
        return false;
    }
    let down = |key: VIRTUAL_KEY| GetKeyState(key.0 as i32) < 0;
    let shift = down(VK_SHIFT);
    match VIRTUAL_KEY(message.wParam.0 as u16) {
        VK_RETURN | VK_F3 => step(hwnd, shift),
        VK_ESCAPE => close(hwnd),
        VK_TAB => leave(hwnd, shift),
        // Ctrl+F again, and Ctrl+A, which the classic EDIT ignores.
        key if (key.0 == 0x46 || key.0 == 0x41) && down(VK_CONTROL) => {
            SendMessageW(message.hwnd, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
        }
        _ => return false,
    }
    true
}

/// Enter goes to the next match, Shift+Enter to the previous one. While
/// typing has not been searched yet, Enter searches now instead.
unsafe fn step(hwnd: HWND, back: bool) {
    let waiting = with_state(|s| {
        let waiting = s.find.sent != key(s);
        if waiting {
            s.find.due = None;
        }
        waiting
    })
    .unwrap_or(true);
    if !waiting {
        actions::execute(hwnd, if back { Command::FindPrevious } else { Command::FindNext }, true);
    }
}

unsafe fn leave(hwnd: HWND, back: bool) {
    let stay = with_state(|s| {
        let layout = s.layout();
        s.focus = widgets::next_focus(&layout.widgets, Some(WidgetId::FindField), back);
        s.focus_visible = true;
        s.document_ring = s.focus == Some(WidgetId::Document);
        s.focus == Some(WidgetId::FindField)
    })
    .unwrap_or(true);
    if !stay {
        let _ = SetFocus(Some(hwnd));
    }
    invalidate(hwnd);
}

/// Tab and F6 move focus by widget; when it lands on the text box, the EDIT
/// takes the keyboard.
pub(super) unsafe fn follow_focus() {
    let edit = with_state(|s| s.find.edit.filter(|_| s.focus == Some(WidgetId::FindField))).flatten();
    if let Some(edit) = edit {
        let _ = SetFocus(Some(edit));
        SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
    }
}

/// A press outside the bar takes the keyboard from the EDIT. Presses on the
/// bar's buttons leave it there, so typing can go on after Next.
pub(super) unsafe fn blur(hwnd: HWND, x: f32, y: f32) {
    let edit = with_state(|s| {
        let edit = s.find.edit?;
        let layout = s.layout();
        let on_bar = widgets::hit(&layout.widgets, x, y).is_some_and(|w| w.region == Region::FindBar && w.id != WidgetId::FindClose);
        (!on_bar).then_some(edit)
    })
    .flatten();
    if edit.is_some_and(|edit| GetFocus() == edit) {
        let _ = SetFocus(Some(hwnd));
    }
}

pub(super) unsafe fn activate(hwnd: HWND, id: WidgetId) {
    match id {
        WidgetId::FindCase => {
            with_state(|s| {
                s.find.match_case = !s.find.match_case;
                s.find.due = None;
            });
        }
        WidgetId::FindClose => close(hwnd),
        _ => {
            with_state(|s| s.focus = Some(WidgetId::FindField));
            follow_focus();
        }
    }
}

struct Geometry {
    card: Rect,
    icon: Rect,
    field: Rect,
    count: Rect,
    case: Rect,
    previous: Rect,
    next: Rect,
    close: Rect,
}

/// The bar floats at the top right of the document, below any info bar.
fn geometry(state: &State, doc: Rect) -> Option<Geometry> {
    if !state.find.open || (state.pdf.is_none() && state.frame.is_none()) {
        return None;
    }
    let (s, ts) = (state.scale, state.text_scale);
    let (pad, gap, button) = (6.0 * s, 2.0 * s, 32.0 * s);
    let control = control_height(ts) * s;
    let icon_width = 24.0 * s;
    let base = 2.0 * pad + icon_width + 4.0 * button + 6.0 * gap;
    let room = doc.width() - 32.0 * s;
    // A narrow window drops the count before the text box gets too small.
    let count_width = (88.0 * ts * s).min(room - base - 80.0 * s);
    let count_width = if count_width < 48.0 * s { 0.0 } else { count_width };
    let fixed = base + count_width;
    let width = (fixed + 220.0 * s).min(room);
    let field_width = width - fixed;
    if field_width < 80.0 * s {
        return None;
    }
    let top = doc.y0 + 8.0 * s;
    let card = Rect::new(doc.x1 - 16.0 * s - width, top, width, control + 2.0 * pad);
    let mut x = card.x0 + pad;
    let mut take = |w: f32| {
        let r = Rect::new(x, card.y0 + pad, w, control);
        x += w + gap;
        r
    };
    Some(Geometry {
        icon: take(icon_width),
        field: take(field_width),
        count: take(count_width),
        case: take(button),
        previous: take(button),
        next: take(button),
        close: take(button),
        card,
    })
}

pub(super) fn add(state: &State, layout: &mut Layout) {
    let Some(g) = geometry(state, layout.document) else {
        return;
    };
    let modal = state.sheet.is_some();
    let has_hits = !state.find_hits.is_empty();
    for (id, role, rect, label, icon, tooltip, enabled) in [
        (WidgetId::FindField, Role::Field, g.field, "Find in document", None, "", true),
        (WidgetId::FindCase, Role::Button, g.case, "Match case", None, "Match case", true),
        (WidgetId::Command(Command::FindPrevious), Role::Button, g.previous, "Previous match", Some(CHEVRON_UP), "Previous match (Shift+F3)", has_hits),
        (WidgetId::Command(Command::FindNext), Role::Button, g.next, "Next match", Some(CHEVRON_DOWN), "Next match (F3)", has_hits),
        (WidgetId::FindClose, Role::Button, g.close, "Close find", Some(glyph::CANCEL), "Close find (Esc)", true),
    ] {
        let mut widget = plain_widget(id, role, Region::FindBar, rect, label.into());
        widget.glyph = icon;
        widget.tooltip = tooltip.into();
        widget.enabled = enabled;
        widget.focusable = !modal;
        if id == WidgetId::FindCase {
            widget.checked = Some(state.find.match_case);
        }
        layout.widgets.push(widget);
    }
}

pub(super) fn paint(l: &Look, state: &State, layout: &Layout) {
    let Some(g) = geometry(state, layout.document) else {
        return;
    };
    let s = l.s;
    infobar::card(l, g.card);
    l.p.glyph(glyph::SEARCH, g.icon, &l.f.icon, l.t.text_secondary);
    l.p.fill_round(g.field, 4.0 * s, l.t.field);
    l.p.stroke_round(g.field, 4.0 * s, l.t.control_border, l.t.border_width * s.floor().max(1.0));
    let focused = state.find.edit.is_some_and(|edit| unsafe { GetFocus() } == edit);
    if focused {
        l.p.fill_round(Rect { y0: g.field.y1 - 2.0 * s, ..g.field }, s, l.t.accent);
    }
    l.p.text(&count(state), g.count, &l.f.caption, l.t.text_secondary, Align::Center);
    for w in layout.widgets.iter().filter(|w| w.region == Region::FindBar && w.role == Role::Button) {
        icon_button(l, state, w);
        if w.id == WidgetId::FindCase {
            let color = if w.checked == Some(true) && l.t.mode == Mode::Contrast {
                l.t.on_accent
            } else if state.hover == Some(w.id) {
                l.t.hover_text
            } else {
                l.t.text
            };
            l.p.text("Aa", w.rect, &l.f.body, color, Align::Center);
        }
    }
}

/// The match count is a polite live region, so Narrator reads "3 of 41"
/// when typing pauses.
pub(super) fn a11y(state: &State, layout: &Layout, nodes: &mut Vec<(NodeId, Node)>, children: &mut Vec<NodeId>) {
    let Some(g) = geometry(state, layout.document) else {
        return;
    };
    let mut kids = Vec::new();
    let text = count(state);
    if !text.is_empty() && !searching(state) {
        let mut node = Node::new(NodeRole::Label);
        node.set_label(text);
        node.set_bounds(a11y::bounds(g.count));
        node.set_live(Live::Polite);
        nodes.push((COUNT_NODE, node));
        kids.push(COUNT_NODE);
    }
    kids.extend(
        layout.widgets.iter().filter(|w| w.region == Region::FindBar && w.role != Role::Field).map(|w| a11y::node_id(w.id)),
    );
    let mut bar = Node::new(NodeRole::Group);
    bar.set_label("Find");
    bar.set_bounds(a11y::bounds(g.card));
    bar.set_children(kids);
    nodes.push((BAR_NODE, bar));
    children.push(BAR_NODE);
}
