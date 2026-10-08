//! Typing into AcroForm fields on the page, drawing and placing a
//! signature, marking selected text, and free text boxes.
//!
//! Form input goes to the PDFium form session on the document worker
//! (`PdfEngine::form_event`) one batch at a time, and the commits it returns
//! join the recipe. Text boxes use an EDIT child over the page, so IME and
//! the clipboard work there (CLAUDE.md D11).
use super::{
    actions,
    app::{invalidate, schedule, with_state, State},
    commands::Command,
    document::{self, Phase, PointerEvent, PointerKind},
    files::{self, Signature},
    render::{fonts, Align, Painter},
    sheet,
    theme::Rgba,
    view,
    widgets::{Layout, Rect, WidgetId},
    worker::{is_pdf, Edits, Job, Request, Work},
};
use crate::model::{AnnotationKind, FormFeedback, FormInput, ImageEdit, NormRect, PdfAnnotation, PdfEdit};
use crate::pdf::PdfEngine;
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::{EM_LIMITTEXT, EM_SETSEL},
            Input::{Ime::*, KeyboardAndMouse::*},
            WindowsAndMessaging::*,
        },
    },
};

const INK: Rgba = Rgba(0.063, 0.129, 0.38, 1.0);
const PAPER: Rgba = Rgba(1.0, 1.0, 1.0, 1.0);
const PAD_BUTTONS: [&str; 3] = ["Clear", "Cancel", "Save"];
const DOUBLE_CLICK: Duration = Duration::from_millis(500);

thread_local! {
    static EDIT_PROC: Cell<isize> = const { Cell::new(0) };
}

#[derive(Default)]
pub(super) struct Forms {
    /// The file whose form session has the focus.
    path: Option<PathBuf>,
    focus: Option<Field>,
    queued: Vec<(PathBuf, u32, FormInput)>,
    in_flight: bool,
    /// Text went into the focused field; PDFium commits it when the field
    /// loses the focus.
    typed: bool,
    retried: bool,
    starting: bool,
    pressing: Option<u32>,
    surrogate: Option<u16>,
    ime: Option<(i32, i32)>,
    generation: u64,
    annotations: HashMap<u32, Vec<PdfAnnotation>>,
    requested: HashSet<u32>,
    last_press: Option<(Instant, (f32, f32))>,
    pad: Option<Pad>,
    placed: Option<Placed>,
    adjust: Option<Adjust>,
    editor: Option<Editor>,
}

#[derive(Clone, Copy)]
struct Field {
    page: u32,
    rect: NormRect,
    index: Option<u32>,
}

#[derive(Default)]
struct Pad {
    /// Points are `[x, y, pressure]`, with x and y in card widths.
    strokes: Vec<Vec<[f32; 3]>>,
    drawing: bool,
}

/// The last placed signature. It moves and resizes until the next click
/// elsewhere, by replacing its edits at the end of the recipe.
struct Placed {
    path: PathBuf,
    page: u32,
    rect: NormRect,
    before: usize,
    signature: Signature,
}

struct Adjust {
    resize: bool,
    start: (f32, f32),
    rect: NormRect,
    page: [f32; 2],
    preview: NormRect,
}

struct Editor {
    edit: HWND,
    parent: HWND,
    font: HFONT,
    font_px: i32,
    at: Rect,
    path: PathBuf,
    page: u32,
    rect: NormRect,
    existing: Option<(u32, String)>,
}

impl Forms {
    /// True when typed text is not in the recipe yet: in a form field or an
    /// open text box. `None` asks about every file.
    pub(super) fn unsaved(&self, path: Option<&Path>) -> bool {
        let matches = |p: Option<&Path>| path.is_none() || p == path;
        (self.typed && matches(self.path.as_deref())) || self.editor.as_ref().is_some_and(|e| matches(Some(&e.path)))
    }
}

pub(super) enum Op {
    Input { page: u32, inputs: Vec<FormInput> },
    Annotations { page: u32 },
}

pub(super) enum Reply {
    Input { page: u32, inputs: Vec<FormInput>, feedback: FormFeedback, error: Option<String> },
    Annotations { page: u32, result: Result<Vec<PdfAnnotation>, String> },
}

/// Document worker side. `path` is the open file and `source` the bytes the
/// worker reads, which become the opened snapshot after the first autosave.
pub(super) fn run(engine: Option<&mut PdfEngine>, path: &Path, source: &Path, edits: &[PdfEdit], op: Op) -> Reply {
    let Some(engine) = engine else {
        let error = "PDF support could not load.".to_string();
        return match op {
            Op::Input { page, inputs } => Reply::Input { page, inputs, feedback: FormFeedback::default(), error: Some(error) },
            Op::Annotations { page } => Reply::Annotations { page, result: Err(error) },
        };
    };
    engine.alias_password(path, source);
    match op {
        Op::Annotations { page } => Reply::Annotations { page, result: engine.annotations(source, page, edits) },
        Op::Input { page, inputs } => {
            let mut feedback = FormFeedback::default();
            let mut edits = edits.to_vec();
            // A session read from the file before its snapshot existed keeps
            // typed text under the file's own path when it closes.
            if path != source {
                feedback.commits = engine.take_form_commits(path);
                edits.extend(feedback.commits.iter().cloned());
            }
            let mut error = None;
            for input in &inputs {
                match engine.form_event(source, page, *input, &edits) {
                    Ok(next) => {
                        edits.extend(next.commits.iter().cloned());
                        feedback.commits.extend(next.commits);
                        feedback.focus = next.focus;
                        feedback.redraw |= next.redraw;
                    }
                    Err(e) => {
                        feedback.focus = None;
                        error = Some(e);
                        break;
                    }
                }
            }
            Reply::Input { page, inputs, feedback, error }
        }
    }
}

fn request(state: &State, path: PathBuf, page: u32) -> Request {
    Request {
        generation: state.generation,
        path,
        page,
        delta: 0,
        width: 1,
        height: 1,
        sessions: state.sessions.clone(),
        sources: state.opened_sources(),
    }
}

fn send(state: &mut State, page: u32, input: FormInput) {
    let Some(path) = state.forms.path.clone() else {
        return;
    };
    if matches!(input, FormInput::Char(_) | FormInput::Key { code: 0x2E, .. }) {
        state.forms.typed = true;
    }
    let queued = &mut state.forms.queued;
    if matches!(input, FormInput::PointerMove { .. }) && matches!(queued.last(), Some((_, _, FormInput::PointerMove { .. }))) {
        queued.pop();
    }
    queued.push((path, page, input));
    pump(state);
}

/// Sends queued input as one batch. The next batch waits for the reply,
/// because it must carry the recipe with this batch's commits.
fn pump(state: &mut State) {
    if state.forms.in_flight {
        return;
    }
    let Some((path, page)) = state.forms.queued.first().map(|(path, page, _)| (path.clone(), *page)) else {
        return;
    };
    let count = state.forms.queued.iter().take_while(|(p, n, _)| *p == path && *n == page).count();
    let inputs = state.forms.queued.drain(..count).map(|(_, _, input)| input).collect();
    let request = request(state, path, page);
    // Waiting view work still carries the recipe from before this batch.
    // Rendering it after the batch would close the form session.
    state.workers.request_with_sources(Vec::new(), &state.opened_sources());
    state.sent.clear();
    if state.send(Job::Forms(request, Op::Input { page, inputs })) {
        state.forms.in_flight = true;
    } else {
        state.forms.queued.clear();
        state.status = "The PDF worker stopped. Close Peekaboo and reopen the file.".into();
    }
}

fn visible_pages(state: &State, layout: &Layout) -> Vec<u32> {
    let Some(g) = document::geometry_in(state, layout.document) else {
        return Vec::new();
    };
    g.layout.pages[view::visible(&g.layout, g.top, g.top + g.doc.height())].iter().map(|(page, _)| *page).collect()
}

/// Runs before the view asks for tiles. Returns true while a form batch is
/// in flight, so no view work goes out with the recipe from before it.
pub(super) fn sync(state: &mut State, layout: &Layout) -> bool {
    if state.forms.editor.as_ref().is_some_and(|e| state.path.as_ref() != Some(&e.path)) {
        unsafe { close_editor(state, true) };
    }
    if state.forms.generation != state.generation {
        state.forms.generation = state.generation;
        state.forms.annotations.clear();
        state.forms.requested.clear();
        // A new recipe reopens the document; the engine keeps the text typed
        // into the field until a form event collects it.
        if let Some(field) = state.forms.focus.take() {
            send(state, field.page, FormInput::Blur);
        }
    }
    if state.forms.in_flight {
        return true;
    }
    let showing = state.pdf.as_ref().is_some_and(|v| Some(&v.path) == state.path.as_ref());
    if !showing || state.pending {
        return false;
    }
    for page in visible_pages(state, layout) {
        if state.forms.annotations.contains_key(&page) || !state.forms.requested.insert(page) {
            continue;
        }
        let Some(path) = state.path.clone() else {
            break;
        };
        let request = request(state, path, page);
        if !state.send(Job::Forms(request, Op::Annotations { page })) {
            state.forms.requested.remove(&page);
            break;
        }
    }
    false
}

pub(super) fn received(state: &mut State, generation: u64, path: PathBuf, reply: Reply) {
    match reply {
        Reply::Annotations { page, result } => {
            if generation == state.forms.generation && state.path.as_ref() == Some(&path) {
                state.forms.requested.remove(&page);
                state.forms.annotations.insert(page, result.unwrap_or_default());
            }
        }
        Reply::Input { page, inputs, feedback, error } => {
            state.forms.in_flight = false;
            commit(state, &path, feedback.commits);
            let active = state.forms.path.as_ref() == Some(&path) && state.path.as_ref() == Some(&path);
            let starting = std::mem::take(&mut state.forms.starting);
            if let Some(error) = error {
                state.forms.focus = None;
                state.forms.typed = false;
                state.forms.queued.retain(|(p, _, _)| *p != path);
                state.status = error;
            } else if active {
                // Only typing lost the focus: the document reopened under the
                // session (the first autosave switches to the opened snapshot).
                let typing = inputs.iter().all(|input| match input {
                    FormInput::Char(c) => *c != '\r',
                    FormInput::Key { code, .. } => *code != 0x09,
                    _ => false,
                });
                let refocus = state.forms.focus.and_then(|f| f.index).filter(|_| typing && feedback.focus.is_none() && !state.forms.retried);
                match refocus {
                    Some(index) => {
                        state.forms.retried = true;
                        let end = FormInput::Key { code: 0x23, shift: false, ctrl: true, alt: false };
                        let mut replay: Vec<_> =
                            [FormInput::Focus(index), end].into_iter().chain(inputs).map(|input| (path.clone(), page, input)).collect();
                        replay.append(&mut state.forms.queued);
                        state.forms.queued = replay;
                    }
                    None => {
                        state.forms.retried = false;
                        let stayed = matches!((state.forms.focus, feedback.focus), (Some(a), Some((page, rect))) if a.page == page && near(a.rect, rect));
                        if !stayed {
                            state.forms.typed = false;
                        }
                        set_focus(state, feedback.focus);
                        if starting && state.forms.focus.is_none() {
                            state.status = "This PDF has no fields to fill. Use Text box to add text.".into();
                        }
                    }
                }
                if feedback.redraw {
                    redraw(state, page);
                    if let Some(field) = state.forms.focus.filter(|f| f.page != page) {
                        redraw(state, field.page);
                    }
                }
            } else if state.forms.path.as_ref() == Some(&path) && feedback.focus.is_none() {
                state.forms.typed = false;
            }
            pump(state);
        }
    }
}

/// Adds form commits to the recipe. The open document already holds them,
/// so the view takes the new recipe without reopening the file.
fn commit(state: &mut State, path: &PathBuf, commits: Vec<PdfEdit>) {
    if commits.is_empty() || !state.tabs.contains(path) {
        return;
    }
    let edits = state.sessions.entry(path.clone()).or_default();
    edits.pdf.extend(commits);
    edits.dirty = true;
    let recipe = Arc::new(edits.pdf.clone());
    state.edited_for_save(path, Instant::now());
    if let Some(v) = state.pdf.as_mut().filter(|v| &v.path == path) {
        let sizes = v.sizes.clone();
        if v.update(sizes, recipe, &mut state.cache) {
            state.sent.clear();
        }
    }
}

fn redraw(state: &mut State, page: u32) {
    let Some(doc) = state.pdf.as_ref().map(|v| v.doc) else {
        return;
    };
    for (key, tile) in state.cache.values_mut() {
        if key.doc == doc && matches!(key.work, Work::Tile { page: p, .. } | Work::Thumb { page: p } if p == page) {
            tile.fresh = false;
        }
    }
    state.sent.clear();
}

fn near(a: NormRect, b: NormRect) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
}

fn set_focus(state: &mut State, focus: Option<(u32, NormRect)>) {
    let index = |page: u32, rect: NormRect| {
        state.forms.annotations.get(&page)?.iter().find(|a| a.kind == "Widget" && near(a.rect, rect)).map(|a| a.index)
    };
    let field = focus.map(|(page, rect)| Field { page, rect, index: index(page, rect) });
    state.forms.focus = field;
    let Some(field) = field else {
        return;
    };
    let doc = state.layout().document;
    let shown = document::text_rect(state, field.page, field.rect).is_some_and(|r| r.y0 >= doc.y0 && r.y1 <= doc.y1);
    if !shown {
        document::go_to_page(state, field.page, true);
    }
}

fn contains(rect: NormRect, at: [f32; 2]) -> bool {
    at[0] >= rect[0] && at[0] <= rect[2] && at[1] >= rect[1] && at[1] <= rect[3]
}

fn annotation_at(state: &State, page: u32, at: [f32; 2], kind: &str) -> Option<PdfAnnotation> {
    state.forms.annotations.get(&page)?.iter().rev().find(|a| a.kind == kind && contains(a.rect, at)).cloned()
}

/// A client point in points from the top-left corner of the displayed
/// page, as the form session takes it. Points off the page still map.
fn page_point(state: &State, page: u32, x: f32, y: f32) -> Option<(f32, f32)> {
    let size = state.pdf.as_ref()?.sizes.get(page as usize)?;
    let r = document::text_rect(state, page, [0.0, 0.0, 1.0, 1.0])?;
    Some(((x - r.x0) / r.width().max(1.0) * size[0], (y - r.y0) / r.height().max(1.0) * size[1]))
}

fn double_click(state: &State, e: &PointerEvent) -> bool {
    let slop = 4.0 * state.scale;
    state.forms.last_press.is_some_and(|(time, at)| {
        time.elapsed() <= DOUBLE_CLICK && (at.0 - e.x).abs() <= slop && (at.1 - e.y).abs() <= slop
    })
}

fn focused(state: &State) -> Option<Field> {
    state.forms.focus.filter(|_| state.forms.path.is_some() && state.forms.path == state.path)
}

fn blur(state: &mut State) {
    if let Some(field) = state.forms.focus.take() {
        send(state, field.page, FormInput::Blur);
    }
}

/// True when a press starts an edit, so the save target is set first.
pub(super) fn needs_consent(state: &State, e: &PointerEvent) -> bool {
    if e.phase != Phase::Down || state.forms.pad.is_some() {
        return false;
    }
    let Some((page, at, _)) = document::text_point(state, e.x, e.y) else {
        return false;
    };
    if state.signature.is_some() {
        return true;
    }
    if state.markup.is_some() || state.crop || state.zoom_select || state.pdf.is_none() {
        return false;
    }
    focused(state).is_some_and(|f| f.page == page)
        || annotation_at(state, page, at, "Widget").is_some()
        || (double_click(state, e) && annotation_at(state, page, at, "FreeText").is_some())
}

/// Pointer input on the document. Returns None when the view should
/// handle the event as usual.
pub(super) unsafe fn pointer(hwnd: HWND, state: &mut State, e: &PointerEvent) -> Option<bool> {
    if state.forms.pad.is_some() {
        return Some(pad_pointer(hwnd, state, e));
    }
    if state.forms.editor.is_some() {
        if e.phase == Phase::Down {
            close_editor(state, true);
        }
        return Some(true);
    }
    if state.forms.adjust.is_some() || (e.phase == Phase::Down && state.forms.placed.is_some()) {
        if let Some(repaint) = adjust_pointer(hwnd, state, e) {
            return Some(repaint);
        }
    }
    if let Some(repaint) = super::marks::pointer(hwnd, state, e) {
        return Some(repaint);
    }
    if state.signature.is_some() {
        if e.phase == Phase::Down {
            place_signature(hwnd, state, e.x, e.y);
        }
        return Some(true);
    }
    if state.markup == Some(AnnotationKind::Text) {
        if e.phase == Phase::Down {
            text_box(hwnd, state, e.x, e.y);
        }
        return Some(true);
    }
    if state.markup.is_some() || state.crop || state.zoom_select {
        return None;
    }
    form_pointer(hwnd, state, e)
}

unsafe fn form_pointer(hwnd: HWND, state: &mut State, e: &PointerEvent) -> Option<bool> {
    match e.phase {
        Phase::Down => {
            let hit = document::text_point(state, e.x, e.y).filter(|_| state.pdf.is_some());
            let double = double_click(state, e);
            state.forms.last_press = Some((Instant::now(), (e.x, e.y)));
            let Some((page, at, _)) = hit else {
                blur(state);
                return None;
            };
            if double {
                if let (Some(found), Some(path)) = (annotation_at(state, page, at, "FreeText"), state.path.clone()) {
                    blur(state);
                    open_editor(hwnd, state, path, page, found.rect, Some((found.index, found.contents)));
                    return Some(true);
                }
            }
            let mine = focused(state).is_some_and(|f| f.page == page);
            if !mine && annotation_at(state, page, at, "Widget").is_none() {
                blur(state);
                return None;
            }
            let (x, y) = page_point(state, page, e.x, e.y)?;
            if !mine {
                blur(state);
            }
            state.forms.path = state.path.clone();
            state.forms.pressing = Some(page);
            state.drag = Some((e.x, e.y));
            if e.kind == PointerKind::Mouse {
                SetCapture(hwnd);
            }
            send(state, page, FormInput::PointerDown { x, y });
            Some(true)
        }
        Phase::Move => {
            let page = state.forms.pressing?;
            if let Some((x, y)) = page_point(state, page, e.x, e.y) {
                send(state, page, FormInput::PointerMove { x, y });
            }
            Some(false)
        }
        Phase::Up | Phase::Cancel => {
            let page = state.forms.pressing.take()?;
            state.drag = None;
            let _ = ReleaseCapture();
            if let Some((x, y)) = page_point(state, page, e.x, e.y) {
                send(state, page, FormInput::PointerUp { x, y });
            }
            Some(true)
        }
    }
}

/// Keys for the focused field, the signature pad, or a placed signature.
pub(super) unsafe fn key_down(hwnd: HWND, vk: u16, ctrl: bool, shift: bool, alt: bool) -> bool {
    with_state(|s| key(hwnd, s, vk, ctrl, shift, alt)).unwrap_or(false)
}

unsafe fn key(hwnd: HWND, s: &mut State, vk: u16, ctrl: bool, shift: bool, alt: bool) -> bool {
    if !matches!(s.focus, Some(WidgetId::Document) | None) {
        return false;
    }
    let key = VIRTUAL_KEY(vk);
    if s.forms.pad.is_some() {
        if ctrl || alt {
            return false;
        }
        match key {
            VK_RETURN => save_pad(s),
            VK_ESCAPE => close_pad(s),
            VK_DELETE | VK_BACK => {
                if let Some(pad) = s.forms.pad.as_mut() {
                    pad.strokes.clear();
                }
            }
            _ => {}
        }
        return true;
    }
    if s.forms.placed.is_some() && !ctrl && !alt && matches!(key, VK_ESCAPE | VK_RETURN) {
        s.forms.placed = None;
        s.forms.adjust = None;
        return true;
    }
    if !ctrl && !alt && super::marks::key(hwnd, s, key) {
        return true;
    }
    let Some(field) = focused(s) else {
        return false;
    };
    if alt || (0x70..=0x87).contains(&vk) {
        return false;
    }
    if ctrl {
        if key == VK_V {
            paste(hwnd, s, field.page);
            return true;
        }
        return false;
    }
    match key {
        VK_ESCAPE => {
            blur(s);
            s.status = s.subtitle();
        }
        VK_TAB | VK_PRIOR | VK_NEXT | VK_END | VK_HOME | VK_LEFT | VK_UP | VK_RIGHT | VK_DOWN | VK_DELETE => {
            send(s, field.page, FormInput::Key { code: vk as u32, shift, ctrl: false, alt: false });
        }
        _ => {}
    }
    true
}

unsafe fn paste(hwnd: HWND, s: &mut State, page: u32) {
    let Some(text) = files::read_clipboard(hwnd) else {
        return;
    };
    // Enter would end a one-line field, so line breaks paste as spaces.
    for c in text.chars().filter(|c| *c != '\r').take(10_000) {
        let c = if c == '\n' || c == '\t' { ' ' } else { c };
        if !c.is_control() {
            send(s, page, FormInput::Char(c));
        }
    }
}

/// WM_CHAR for the focused field. Returns true when the field took it.
pub(super) fn char_input(code: u32) -> bool {
    with_state(|s| {
        let Some(field) = focused(s).filter(|_| matches!(s.focus, Some(WidgetId::Document) | None)) else {
            return false;
        };
        // WM_CHAR carries UTF-16 units, so characters above U+FFFF arrive in two messages.
        let unit = code as u16;
        let c = match unit {
            0xD800..=0xDBFF => {
                s.forms.surrogate = Some(unit);
                return true;
            }
            0xDC00..=0xDFFF => s.forms.surrogate.take().and_then(|high| char::decode_utf16([high, unit]).next()?.ok()),
            _ => char::from_u32(code),
        };
        if let Some(c) = c.filter(|c| matches!(c, '\u{8}' | '\r') || !c.is_control()) {
            send(s, field.page, FormInput::Char(c));
        }
        true
    })
    .unwrap_or(false)
}

/// Fill form: the first field of the current page takes the focus, as Tab
/// does in PDFium.
pub(super) unsafe fn start_form(hwnd: HWND) {
    let Some(path) = with_state(|s| s.path.clone()).flatten() else {
        return;
    };
    if !actions::prepare_save(hwnd, &path) {
        return;
    }
    with_state(|s| {
        if s.path.as_ref() != Some(&path) {
            return;
        }
        s.markup = None;
        s.signature = None;
        s.crop = false;
        s.zoom_select = false;
        s.focus = Some(WidgetId::Document);
        let page = focused(s).map_or(s.page, |f| f.page);
        s.forms.path = Some(path);
        s.forms.starting = true;
        send(s, page, FormInput::Key { code: 0x09, shift: false, ctrl: false, alt: false });
        s.status = "Tab and Shift+Tab move between fields. Escape finishes.".into();
    });
    invalidate(hwnd);
}

/// The markup tools that act at once. Returns false when the tool should
/// start as usual.
pub(super) unsafe fn tool(hwnd: HWND, command: Command) -> bool {
    match command {
        Command::Highlight | Command::Underline | Command::Strikethrough => mark_selection(hwnd, command),
        Command::PlaceSignature => {
            match files::signatures().into_iter().next() {
                Some(signature) => with_state(|s| start_signing(s, signature)),
                None => with_state(open_pad),
            };
            invalidate(hwnd);
            true
        }
        _ => {
            with_state(|s| s.forms.pad = None);
            false
        }
    }
}

pub(super) unsafe fn new_signature(hwnd: HWND) {
    with_state(open_pad);
    invalidate(hwnd);
}

/// The Sign button: pick a saved signature to place, or create one.
pub(super) unsafe fn sign_menu(hwnd: HWND, keyboard: bool) {
    let saved = files::signatures();
    let Some(ctx) = with_state(|s| s.ctx()) else {
        return;
    };
    let items = super::commands::sign_menu(&ctx, &saved);
    match actions::popup(hwnd, items, Some(WidgetId::Command(Command::SignMenu)), None, keyboard) {
        Some(super::commands::Pick::Index(index)) => {
            if let Some(signature) = saved.into_iter().nth(index) {
                with_state(|s| start_signing(s, signature));
            }
            invalidate(hwnd);
        }
        Some(super::commands::Pick::Command(command)) => actions::execute(hwnd, command, keyboard),
        None => {}
    }
}

/// Highlight, underline, or strike out the selected text: one annotation
/// per page, with one quad per line.
unsafe fn mark_selection(hwnd: HWND, command: Command) -> bool {
    let kind = match command {
        Command::Highlight => AnnotationKind::Highlight,
        Command::Underline => AnnotationKind::Underline,
        _ => AnnotationKind::Strikeout,
    };
    let Some((pdf, mut marks)) = with_state(|s| {
        let selection = s.text_selection.filter(|x| !x.is_empty())?;
        let (start, end) = selection.span();
        let marks: Vec<(u32, Vec<NormRect>)> = s
            .text_layers
            .iter()
            .filter(|(page, _)| (start.page..=end.page).contains(*page))
            .filter_map(|(page, layer)| {
                let rects = super::text::rects(layer, selection.range(*page, layer.boxes.len())?);
                (!rects.is_empty()).then_some((*page, rects))
            })
            .collect();
        Some((s.is_pdf(), marks))
    })
    .flatten() else {
        return false;
    };
    if marks.is_empty() {
        return false;
    }
    marks.sort_by_key(|(page, _)| *page);
    actions::edit(hwnd, |s, edits| {
        s.text_selection = None;
        for (page, rects) in marks {
            let corners = |r: &NormRect| [[r[0], r[1]], [r[2], r[3]]];
            if pdf {
                let points = rects.iter().flat_map(corners).collect();
                edits.pdf.push(PdfEdit::Annotate { page, kind, points, text: String::new() });
            } else {
                edits.image.extend(rects.iter().map(|r| ImageEdit::Annotate { kind, points: corners(r).to_vec(), text: String::new() }));
            }
        }
    });
    true
}

/// Adds edits to a file's recipe once its save target is set.
unsafe fn push(hwnd: HWND, state: &mut State, path: &PathBuf, change: impl FnOnce(&mut Edits)) {
    if !state.tabs.contains(path) {
        return;
    }
    let edits = state.sessions.entry(path.clone()).or_default();
    change(edits);
    edits.dirty = true;
    state.edited_for_save(path, Instant::now());
    if state.path.as_ref() == Some(path) {
        schedule(hwnd, state, 0);
    }
}

fn open_pad(state: &mut State) {
    state.markup = None;
    state.signature = None;
    state.crop = false;
    state.zoom_select = false;
    state.forms.placed = None;
    state.forms.pad = Some(Pad::default());
    state.focus = Some(WidgetId::Document);
    state.status = "Draw your signature with a mouse, pen, or finger. Enter saves it on this PC.".into();
}

fn close_pad(state: &mut State) {
    state.forms.pad = None;
    state.status = "Signature not saved.".into();
}

fn save_pad(state: &mut State) {
    let Some(pad) = state.forms.pad.as_ref() else {
        return;
    };
    match files::store_signature(&pad.strokes) {
        Ok(signature) => {
            state.forms.pad = None;
            start_signing(state, signature);
            state.status = "Signature saved on this PC. Click where it goes.".into();
        }
        Err(error) => state.status = error,
    }
}

fn start_signing(state: &mut State, signature: Signature) {
    state.markup = None;
    state.crop = false;
    state.zoom_select = false;
    state.marks.deselect();
    state.forms.placed = None;
    state.signature = Some(signature);
    state.set_markup(true);
    state.status = "Click where your signature goes. Escape cancels.".into();
}

fn pad_card(doc: Rect, s: f32) -> Rect {
    let w = (520.0 * s).min(doc.width() - 32.0 * s).max(1.0);
    let h = (w * 0.5).min(doc.height() - 32.0 * s).max(1.0);
    Rect::new(doc.x0 + (doc.width() - w) / 2.0, doc.y0 + (doc.height() - h) / 2.0, w, h)
}

fn pad_buttons(card: Rect, s: f32) -> [Rect; 3] {
    let (w, h, gap) = (80.0 * s, 32.0 * s, 8.0 * s);
    let (right, y) = (card.x1 - 12.0 * s, card.y1 - h - 12.0 * s);
    [
        Rect::new(right - 3.0 * w - 2.0 * gap, y, w, h),
        Rect::new(right - 2.0 * w - gap, y, w, h),
        Rect::new(right - w, y, w, h),
    ]
}

unsafe fn pad_pointer(hwnd: HWND, state: &mut State, e: &PointerEvent) -> bool {
    let s = state.scale;
    let card = pad_card(state.layout().document, s);
    let buttons = pad_buttons(card, s);
    let point = [(e.x - card.x0) / card.width(), (e.y - card.y0) / card.width(), e.pressure.unwrap_or(0.5)];
    if e.phase == Phase::Down {
        match buttons.iter().position(|b| b.contains(e.x, e.y)) {
            Some(1) => close_pad(state),
            Some(2) => save_pad(state),
            button => {
                let Some(pad) = state.forms.pad.as_mut() else {
                    return false;
                };
                if button.is_some() {
                    pad.strokes.clear();
                } else if card.contains(e.x, e.y) && pad.strokes.iter().map(Vec::len).sum::<usize>() < 20_000 {
                    pad.strokes.push(vec![point]);
                    pad.drawing = true;
                    state.drag = Some((e.x, e.y));
                    if e.kind == PointerKind::Mouse {
                        SetCapture(hwnd);
                    }
                }
            }
        }
        return true;
    }
    let Some(pad) = state.forms.pad.as_mut() else {
        return false;
    };
    match e.phase {
        Phase::Down => true,
        Phase::Move => {
            if pad.drawing {
                if let Some(stroke) = pad.strokes.last_mut().filter(|stroke| stroke.len() < 20_000) {
                    stroke.push(point);
                }
            }
            pad.drawing
        }
        Phase::Up | Phase::Cancel => {
            if std::mem::take(&mut pad.drawing) {
                state.drag = None;
                let _ = ReleaseCapture();
            }
            true
        }
    }
}

/// A PDF gets one Sign edit; an image gets one Ink annotation per stroke.
fn signature_edits(pdf: bool, page: u32, rect: NormRect, signature: &Signature) -> (Vec<PdfEdit>, Vec<ImageEdit>) {
    if pdf {
        return (vec![PdfEdit::Sign { page, rect, strokes: signature.strokes.clone() }], Vec::new());
    }
    let ink = signature
        .strokes
        .iter()
        .map(|stroke| {
            let mut points: Vec<[f32; 2]> =
                stroke.iter().map(|p| [rect[0] + p[0] * (rect[2] - rect[0]), rect[1] + p[1] * (rect[3] - rect[1])]).collect();
            if points.len() == 1 {
                points.push(points[0]);
            }
            ImageEdit::Annotate { kind: AnnotationKind::Ink, points, text: String::new() }
        })
        .collect();
    (Vec::new(), ink)
}

/// True while the placed signature is still the end of its file's recipe.
fn placed_live(state: &State, placed: &Placed) -> bool {
    let pdf = is_pdf(&placed.path);
    let (p, i) = signature_edits(pdf, placed.page, placed.rect, &placed.signature);
    state.sessions.get(&placed.path).is_some_and(|edits| {
        if pdf {
            edits.pdf.get(placed.before..) == Some(&p[..])
        } else {
            edits.image.get(placed.before..) == Some(&i[..])
        }
    })
}

unsafe fn place_signature(hwnd: HWND, state: &mut State, x: f32, y: f32) {
    let (Some(path), Some((page, at, size))) = (state.path.clone(), document::text_point(state, x, y)) else {
        return;
    };
    let Some(signature) = state.signature.take() else {
        return;
    };
    let mut width = size[0] * 0.3;
    let mut height = width * signature.aspect;
    if height > size[1] * 0.4 {
        height = size[1] * 0.4;
        width = height / signature.aspect;
    }
    let (w, h) = ((width / size[0]).min(1.0), (height / size[1]).min(1.0));
    let left = (at[0] - w / 2.0).clamp(0.0, 1.0 - w);
    let top = (at[1] - h / 2.0).clamp(0.0, 1.0 - h);
    let rect = [left, top, left + w, top + h];
    let pdf = is_pdf(&path);
    let before = state.sessions.get(&path).map_or(0, |e| if pdf { e.pdf.len() } else { e.image.len() });
    let (pdf_edits, image_edits) = signature_edits(pdf, page, rect, &signature);
    state.forms.placed = Some(Placed { path: path.clone(), page, rect, before, signature });
    push(hwnd, state, &path, |edits| {
        edits.pdf.extend(pdf_edits);
        edits.image.extend(image_edits);
    });
    state.status = "Signature added. Drag it to move it, or drag its corner to resize it.".into();
}

fn handle(r: Rect, s: f32) -> Rect {
    let size = 10.0 * s;
    Rect::new(r.x1 - size / 2.0, r.y1 - size / 2.0, size, size)
}

unsafe fn adjust_pointer(hwnd: HWND, state: &mut State, e: &PointerEvent) -> Option<bool> {
    match e.phase {
        Phase::Down => {
            let placed = state.forms.placed.as_ref()?;
            let screen = document::text_rect(state, placed.page, placed.rect);
            let page = document::text_rect(state, placed.page, [0.0, 0.0, 1.0, 1.0]);
            let (Some(r), Some(page), true) = (screen, page, state.path.as_ref() == Some(&placed.path) && placed_live(state, placed)) else {
                state.forms.placed = None;
                return None;
            };
            let resize = handle(r, state.scale).inset(-4.0 * state.scale).contains(e.x, e.y);
            if !resize && !r.contains(e.x, e.y) {
                state.forms.placed = None;
                return None;
            }
            let rect = placed.rect;
            state.forms.adjust = Some(Adjust { resize, start: (e.x, e.y), rect, page: [page.width(), page.height()], preview: rect });
            state.drag = Some((e.x, e.y));
            if e.kind == PointerKind::Mouse {
                SetCapture(hwnd);
            }
            Some(true)
        }
        Phase::Move => {
            let min = 24.0 * state.scale;
            let a = state.forms.adjust.as_mut()?;
            let (dx, dy) = ((e.x - a.start.0) / a.page[0], (e.y - a.start.1) / a.page[1]);
            let [l, t, r, b] = a.rect;
            a.preview = if a.resize {
                // Width and height in pixels keep the signature's shape.
                let aspect = (b - t) * a.page[1] / ((r - l) * a.page[0]).max(1e-6);
                let mut w = ((r - l + dx) * a.page[0]).max(min).min((1.0 - l) * a.page[0]);
                let mut h = w * aspect;
                if h > (1.0 - t) * a.page[1] {
                    h = (1.0 - t) * a.page[1];
                    w = h / aspect.max(1e-6);
                }
                [l, t, l + w / a.page[0], t + h / a.page[1]]
            } else {
                let x = (l + dx).clamp(0.0, 1.0 - (r - l));
                let y = (t + dy).clamp(0.0, 1.0 - (b - t));
                [x, y, x + r - l, y + b - t]
            };
            Some(true)
        }
        Phase::Up | Phase::Cancel => {
            let a = state.forms.adjust.take()?;
            state.drag = None;
            let _ = ReleaseCapture();
            if e.phase == Phase::Up && a.preview != a.rect {
                move_signature(hwnd, state, a.preview);
            }
            Some(true)
        }
    }
}

unsafe fn move_signature(hwnd: HWND, state: &mut State, rect: NormRect) {
    let Some(placed) = state.forms.placed.take() else {
        return;
    };
    if !placed_live(state, &placed) {
        state.status = "The signature can move only until you make another edit.".into();
        return;
    }
    let pdf = is_pdf(&placed.path);
    let (p, i) = signature_edits(pdf, placed.page, rect, &placed.signature);
    let (path, before) = (placed.path.clone(), placed.before);
    state.forms.placed = Some(Placed { rect, ..placed });
    push(hwnd, state, &path, |edits| {
        if pdf {
            edits.pdf.truncate(before);
            edits.pdf.extend(p);
        } else {
            edits.image.truncate(before);
            edits.image.extend(i);
        }
    });
}

unsafe fn text_box(hwnd: HWND, state: &mut State, x: f32, y: f32) {
    let (Some(path), Some((page, at, _))) = (state.path.clone(), document::text_point(state, x, y)) else {
        return;
    };
    if let Some(found) = annotation_at(state, page, at, "FreeText").filter(|_| state.pdf.is_some()) {
        open_editor(hwnd, state, path, page, found.rect, Some((found.index, found.contents)));
        return;
    }
    // Three lines of 12 pt text on a PDF; images draw text 60% of their width wide.
    let (w, h) = match state.pdf.as_ref().and_then(|v| v.sizes.get(page as usize)) {
        Some([pw, ph]) => ((220.0 / pw).min(1.0), (46.0 / ph).min(1.0)),
        None => (0.6, 0.15),
    };
    let left = at[0].min(1.0 - w).max(0.0);
    let top = at[1].min(1.0 - h).max(0.0);
    open_editor(hwnd, state, path, page, [left, top, left + w, top + h], None);
}

/// Pixel height of the text box font: the text style size on a PDF page,
/// scaled as `imaging` draws it on an image.
fn editor_font_px(state: &State, page: u32) -> i32 {
    let screen = document::text_rect(state, page, [0.0, 0.0, 1.0, 1.0]);
    let points = state.marks.style.size;
    let px = match (state.pdf.as_ref().and_then(|v| v.sizes.get(page as usize)), screen) {
        (Some(size), Some(r)) => points * r.height() / size[1],
        (None, Some(r)) => r.width() * 0.0025 * points,
        _ => 14.0 * state.scale,
    };
    px.round().clamp(6.0, 400.0) as i32
}

unsafe fn editor_font(px: i32) -> HFONT {
    CreateFontW(
        -px,
        0,
        0,
        0,
        FW_NORMAL.0 as i32,
        0,
        0,
        0,
        DEFAULT_CHARSET,
        OUT_DEFAULT_PRECIS,
        CLIP_DEFAULT_PRECIS,
        CLEARTYPE_QUALITY,
        0,
        w!("Arial"),
    )
}

unsafe fn open_editor(hwnd: HWND, state: &mut State, path: PathBuf, page: u32, rect: NormRect, existing: Option<(u32, String)>) {
    let Some(at) = document::text_rect(state, page, rect) else {
        return;
    };
    let font_px = editor_font_px(state, page);
    let text: Vec<u16> = existing.as_ref().map_or(String::new(), |(_, t)| t.replace('\n', "\r\n")).encode_utf16().chain(Some(0)).collect();
    let style = WS_CHILD | WS_VISIBLE | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32);
    let Ok(edit) = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("EDIT"),
        PCWSTR(text.as_ptr()),
        style,
        at.x0 as i32,
        at.y0 as i32,
        at.width().max(40.0) as i32,
        at.height().max(font_px as f32 + 6.0) as i32,
        Some(hwnd),
        None,
        GetModuleHandleW(None).ok().map(Into::into),
        None,
    ) else {
        state.status = "Cannot open the text box.".into();
        return;
    };
    let font = editor_font(font_px);
    SendMessageW(edit, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
    SendMessageW(edit, EM_LIMITTEXT, Some(WPARAM(4000)), None);
    sheet::name_field(edit, "Text box");
    let proc: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT = editor_proc;
    EDIT_PROC.with(|cell| cell.set(SetWindowLongPtrW(edit, GWLP_WNDPROC, proc as isize)));
    state.forms.editor = Some(Editor { edit, parent: hwnd, font, font_px, at, path, page, rect, existing });
    let _ = SetFocus(Some(edit));
    let end = GetWindowTextLengthW(edit).max(0) as usize;
    SendMessageW(edit, EM_SETSEL, Some(WPARAM(end)), Some(LPARAM(end as isize)));
    state.status = "Type the text. Click outside the box or press Ctrl+Enter to finish. Escape cancels.".into();
}

unsafe extern "system" fn editor_proc(edit: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
    match message {
        WM_KEYDOWN if wparam.0 == VK_ESCAPE.0 as usize => {
            finish_editor(edit, false);
            return LRESULT(0);
        }
        WM_KEYDOWN if wparam.0 == VK_TAB.0 as usize || (wparam.0 == VK_RETURN.0 as usize && ctrl) => {
            finish_editor(edit, true);
            return LRESULT(0);
        }
        // A multi-line EDIT posts WM_CLOSE to its parent on Escape.
        WM_CHAR if matches!(wparam.0, 0x1B | 0x09 | 0x0A) => return LRESULT(0),
        _ => {}
    }
    let previous: WNDPROC = std::mem::transmute(EDIT_PROC.with(Cell::get));
    CallWindowProcW(previous, edit, message, wparam, lparam)
}

unsafe fn finish_editor(edit: HWND, commit: bool) {
    let parent = GetParent(edit).unwrap_or_default();
    with_state(|s| close_editor(s, commit));
    invalidate(parent);
}

/// The box around new text: Helvetica lines with room for wide glyphs.
fn fitted(state: &State, editor: &Editor, text: &str) -> NormRect {
    let [left, top, ..] = editor.rect;
    let Some([pw, ph]) = state.pdf.as_ref().filter(|v| v.path == editor.path).and_then(|v| v.sizes.get(editor.page as usize).copied()) else {
        return editor.rect;
    };
    let size = state.marks.style.size;
    let lines = text.lines().count().max(1) as f32;
    let longest = text.lines().map(|l| l.chars().count()).max().unwrap_or(1) as f32;
    let w = ((longest * 0.6 * size + 10.0) / pw).clamp(30.0 / pw, 1.0);
    let h = ((lines * 1.17 * size + 8.0) / ph).min(1.0);
    let (left, top) = (left.min(1.0 - w).max(0.0), top.min(1.0 - h).max(0.0));
    [left, top, left + w, top + h]
}

unsafe fn close_editor(state: &mut State, commit: bool) {
    let Some(editor) = state.forms.editor.take() else {
        return;
    };
    let mut buffer = vec![0u16; GetWindowTextLengthW(editor.edit).max(0) as usize + 1];
    let length = GetWindowTextW(editor.edit, &mut buffer).max(0) as usize;
    let text = String::from_utf16_lossy(&buffer[..length]).replace("\r\n", "\n").trim_end().to_string();
    let _ = SetFocus(Some(editor.parent));
    let _ = DestroyWindow(editor.edit);
    let _ = DeleteObject(editor.font.into());
    state.status = state.subtitle();
    if !commit {
        return;
    }
    let page = editor.page;
    let (pdf_edit, image_edit) = match &editor.existing {
        Some((index, _)) if text.trim().is_empty() => (Some(PdfEdit::DeleteAnnotation { page, index: *index }), None),
        Some((index, old)) if *old != text => (Some(PdfEdit::SetAnnotationText { page, index: *index, text }), None),
        Some(_) => return,
        None if text.trim().is_empty() => return,
        None => {
            let r = fitted(state, &editor, &text);
            let points = vec![[r[0], r[1]], [r[2], r[3]]];
            let (kind, style) = (AnnotationKind::Text, state.marks.style);
            if is_pdf(&editor.path) {
                (Some(PdfEdit::Mark { page, kind, points, text, style }), None)
            } else {
                (None, Some(ImageEdit::Mark { kind, points, text, style }))
            }
        }
    };
    let added = editor.existing.is_none();
    push(editor.parent, state, &editor.path, |edits| {
        edits.pdf.extend(pdf_edit);
        edits.image.extend(image_edit);
    });
    if added {
        super::marks::select_last(state, &editor.path);
    }
}

/// Keeps the text box on its page through scrolling and zooming.
unsafe fn place_editor(state: &mut State) {
    let Some(editor) = state.forms.editor.as_ref() else {
        return;
    };
    let (page, rect) = (editor.page, editor.rect);
    let Some(at) = document::text_rect(state, page, rect) else {
        return;
    };
    let font_px = editor_font_px(state, page);
    let Some(editor) = state.forms.editor.as_mut() else {
        return;
    };
    if font_px != editor.font_px {
        let font = editor_font(font_px);
        SendMessageW(editor.edit, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
        let _ = DeleteObject(editor.font.into());
        editor.font = font;
        editor.font_px = font_px;
    }
    if at != editor.at {
        editor.at = at;
        let (w, h) = (at.width().max(40.0) as i32, at.height().max(font_px as f32 + 6.0) as i32);
        let _ = SetWindowPos(editor.edit, None, at.x0 as i32, at.y0 as i32, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

/// Puts the IME composition window over the focused field.
unsafe fn ime(state: &mut State, r: Rect) {
    let at = (r.x0 as i32 + 2, r.y0 as i32 + 1);
    if state.forms.ime == Some(at) || state.sheet.is_some() {
        return;
    }
    state.forms.ime = Some(at);
    let hwnd = GetFocus();
    let context = ImmGetContext(hwnd);
    if context.0.is_null() {
        return;
    }
    let form = COMPOSITIONFORM { dwStyle: CFS_POINT, ptCurrentPos: POINT { x: at.0, y: at.1 }, rcArea: RECT::default() };
    let _ = ImmSetCompositionWindow(context, &form);
    let _ = ImmReleaseContext(hwnd, context);
}

fn ink_width(s: f32, pressure: f32) -> f32 {
    1.6 * s * (0.5 + pressure)
}

/// Draws the focus ring, the placed signature's frame, and the signature
/// pad over the document.
pub(super) fn paint(p: &Painter, state: &mut State, doc: Rect) {
    let s = state.scale;
    let theme = state.theme;
    unsafe { place_editor(state) };
    if let Some(field) = focused(state) {
        if let Some(r) = document::text_rect(state, field.page, field.rect) {
            p.stroke_round(r.inset(-3.0 * s), 2.0 * s, theme.accent, 2.0 * s);
            unsafe { ime(state, r) };
        }
    }
    if let Some(placed) = state.forms.placed.as_ref().filter(|placed| state.path.as_ref() == Some(&placed.path) && placed_live(state, placed)) {
        let rect = state.forms.adjust.as_ref().map_or(placed.rect, |a| a.preview);
        if let Some(r) = document::text_rect(state, placed.page, rect) {
            if state.forms.adjust.is_some() {
                for stroke in &placed.signature.strokes {
                    let at = |q: &[f32; 3]| (r.x0 + q[0] * r.width(), r.y0 + q[1] * r.height());
                    for pair in stroke.windows(2) {
                        p.line(at(&pair[0]), at(&pair[1]), INK, ink_width(s, (pair[0][2] + pair[1][2]) / 2.0));
                    }
                }
            }
            p.stroke_round(r.inset(-2.0 * s), 0.0, theme.accent, s.max(1.0));
            p.fill(handle(r, s), theme.accent);
        }
    }
    let Some(pad) = state.forms.pad.as_ref() else {
        return;
    };
    let card = pad_card(doc, s);
    let buttons = pad_buttons(card, s);
    p.fill(doc, theme.scrim);
    p.fill_round(card, 8.0 * s, PAPER);
    p.stroke_round(card, 8.0 * s, theme.control_border, s.floor().max(1.0));
    let line = buttons[0].y0 - 28.0 * s;
    p.line((card.x0 + 24.0 * s, line), (card.x1 - 24.0 * s, line), Rgba(0.75, 0.75, 0.75, 1.0), s.max(1.0));
    if let Ok(f) = fonts(s, state.text_scale) {
        if pad.strokes.is_empty() {
            let hint = Rect { x0: card.x0 + 24.0 * s, y0: line - 32.0 * s, x1: card.x1 - 24.0 * s, y1: line - 4.0 * s };
            p.text("Sign here", hint, &f.body, Rgba(0.45, 0.45, 0.45, 1.0), Align::Leading);
        }
        for (index, (r, label)) in buttons.iter().zip(PAD_BUTTONS).enumerate() {
            let primary = index == 2;
            p.fill_round(*r, 4.0 * s, if primary { theme.accent } else { Rgba(0.92, 0.92, 0.92, 1.0) });
            p.text(label, *r, &f.body, if primary { theme.on_accent } else { Rgba(0.1, 0.1, 0.1, 1.0) }, Align::Center);
        }
    }
    let at = |q: &[f32; 3]| (card.x0 + q[0] * card.width(), card.y0 + q[1] * card.width());
    for stroke in &pad.strokes {
        if let [only] = stroke.as_slice() {
            p.line(at(only), at(only), INK, ink_width(s, only[2]));
        }
        for pair in stroke.windows(2) {
            p.line(at(&pair[0]), at(&pair[1]), INK, ink_width(s, (pair[0][2] + pair[1][2]) / 2.0));
        }
    }
}
