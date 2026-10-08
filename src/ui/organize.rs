//! Page organizing in the thumbnail list: drag a thumbnail to reorder it,
//! drag it out of the sidebar to copy the page to Explorer as a PDF, move
//! pages with keys, and insert images or PDFs as pages. File drops from
//! other apps (drop.rs) share the insertion line.
use super::{
    actions,
    app::{file_name, invalidate, with_state, State},
    document::{Phase, PointerEvent, PointerKind},
    files::choose_many,
    render::Painter,
    sheet, sidebar,
    widgets::{self, Layout, Rect, WidgetId},
    worker::{is_pdf, Job, Request, SaveKind},
};
use crate::model::PdfEdit;
use std::{
    cell::Cell,
    path::{Path, PathBuf},
};
use windows::Win32::{Foundation::HWND, UI::Input::KeyboardAndMouse::ReleaseCapture};

/// Pointer travel in epx before a press on a thumbnail becomes a drag, so a
/// click still opens the page.
const DRAG_START: f32 = 4.0;
/// Height in epx of the zone at the list's top and bottom that scrolls it.
const EDGE: f32 = 40.0;
/// Fastest autoscroll, in epx per timer tick.
const SPEED: f32 = 12.0;

#[derive(Default)]
pub(super) struct Organize {
    drag: Option<Drag>,
    /// Client point of files dragged in from another app, while over the thumbnails.
    pub(super) drop_over: Option<(f32, f32)>,
    /// Dropped files and their gap, or None to open them as tabs. Tick runs
    /// the drop, so the source app is not held up by dialogs.
    pub(super) dropped: Option<(Vec<PathBuf>, Option<u32>)>,
}

impl Organize {
    pub(super) fn cancel(&mut self) {
        self.drag = None;
        self.drop_over = None;
    }
}

struct Drag {
    pointer: u32,
    page: u32,
    start: (f32, f32),
    at: (f32, f32),
    active: bool,
    mouse: bool,
    out: Out,
}

/// The temporary PDF of the dragged page, for a drag out to Explorer.
#[derive(PartialEq)]
enum Out {
    Idle,
    Extracting(PathBuf),
    Ready(PathBuf),
    Failed,
}

thread_local! { static DRAGGING_OUT: Cell<bool> = const { Cell::new(false) }; }

/// True during a page drag out, so the window refuses its own drop.
pub(super) fn dragging_out() -> bool {
    DRAGGING_OUT.with(Cell::get)
}

fn thumbnails(s: &State) -> bool {
    s.sidebar_visible() && s.sidebar_tab == 0 && s.pdf.is_some() && s.sheet.is_none()
}

fn rows(s: &State, layout: &Layout) -> Vec<(usize, Rect)> {
    widgets::sidebar_rows(&s.sidebar_list(), layout.sidebar_panel, s.sidebar_scroll[0], s.scale, s.text_scale).0
}

/// The gap nearest `y` among the visible thumbnails: gap k goes before page k.
fn insertion(s: &State, layout: &Layout, y: f32) -> u32 {
    let rows = rows(s, layout);
    let y = y.clamp(layout.sidebar_panel.y0, layout.sidebar_panel.y1);
    let gap = match rows.iter().find(|(_, r)| y < (r.y0 + r.y1) / 2.0) {
        Some((index, _)) => *index,
        None => rows.last().map_or(0, |(index, _)| index + 1),
    };
    gap as u32
}

/// The `PdfEdit::Move` target for page `from` dropped at `gap`, or None
/// when the page stays where it is.
fn move_target(from: u32, gap: u32) -> Option<u32> {
    let to = if gap > from { gap - 1 } else { gap };
    (to != from).then_some(to)
}

pub(super) fn can_insert(path: &Path) -> bool {
    is_pdf(path)
        || path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "gif" | "tif" | "tiff" | "bmp" | "webp" | "heic" | "heif"
            )
        })
}

/// The gap for files dropped at `point`, when it is over the thumbnails.
pub(super) fn drop_gap(s: &State, (x, y): (f32, f32)) -> Option<u32> {
    if !thumbnails(s) || s.pending {
        return None;
    }
    let layout = s.layout();
    layout.sidebar?.contains(x, y).then(|| insertion(s, &layout, y))
}

enum Then {
    Pass,
    Handled,
    Move(u32, u32),
}

/// Thumbnail drags. Returns true when the event belongs to a drag; the
/// window handles everything else, including plain clicks.
pub(super) unsafe fn pointer(hwnd: HWND, e: &PointerEvent) -> bool {
    let then = with_state(|s| match e.phase {
        Phase::Down => {
            arm(s, e);
            Then::Pass
        }
        Phase::Move => drag_to(s, e),
        Phase::Up | Phase::Cancel => release(s, e),
    })
    .unwrap_or(Then::Pass);
    match then {
        Then::Pass => return false,
        Then::Handled => {}
        Then::Move(from, to) => move_page(hwnd, from, to),
    }
    invalidate(hwnd);
    true
}

fn arm(s: &mut State, e: &PointerEvent) {
    s.organize.drag = None;
    if !thumbnails(s) || s.pending || e.kind == PointerKind::Touch {
        return;
    }
    if let Some(WidgetId::SidebarItem(index)) = widgets::hit(&s.layout().widgets, e.x, e.y).map(|w| w.id) {
        s.organize.drag = Some(Drag {
            pointer: e.id,
            page: index as u32,
            start: (e.x, e.y),
            at: (e.x, e.y),
            active: false,
            mouse: e.kind == PointerKind::Mouse,
            out: Out::Idle,
        });
    }
}

fn drag_to(s: &mut State, e: &PointerEvent) -> Then {
    let slop = DRAG_START * s.scale;
    let Some(drag) = s.organize.drag.as_mut().filter(|d| d.pointer == e.id) else {
        return Then::Pass;
    };
    drag.at = (e.x, e.y);
    if !drag.active {
        if (e.x - drag.start.0).abs() < slop && (e.y - drag.start.1).abs() < slop {
            return Then::Pass;
        }
        drag.active = true;
    }
    let (page, extract_now) = (drag.page, drag.mouse && drag.out == Out::Idle);
    s.set_hover(None);
    let outside = !s.layout().sidebar.is_some_and(|r| r.contains(e.x, e.y));
    // The copy for Explorer starts only once the pointer leaves the
    // sidebar, so a reorder costs nothing extra.
    if outside && extract_now {
        extract(s, page);
    }
    s.status = if outside {
        format!("Drop page {} in a folder to save it as a PDF.", page + 1)
    } else {
        format!("Release to move page {}. Drag it out of the sidebar to save it as a PDF.", page + 1)
    };
    Then::Handled
}

fn release(s: &mut State, e: &PointerEvent) -> Then {
    if !s.organize.drag.as_ref().is_some_and(|d| d.pointer == e.id) {
        return Then::Pass;
    }
    let Some(drag) = s.organize.drag.take().filter(|d| d.active) else {
        return Then::Pass;
    };
    if let Out::Ready(path) = &drag.out {
        let _ = std::fs::remove_file(path);
    }
    s.pressed = None;
    unsafe {
        let _ = ReleaseCapture();
    }
    s.status = s.subtitle();
    let layout = s.layout();
    if e.phase == Phase::Up && layout.sidebar.is_some_and(|r| r.contains(e.x, e.y)) {
        if let Some(to) = move_target(drag.page, insertion(s, &layout, e.y)) {
            return Then::Move(drag.page, to);
        }
    }
    Then::Handled
}

/// Asks the document worker for a PDF of `page` in a new temporary folder.
fn extract(s: &mut State, page: u32) {
    let Some(path) = s.path.clone() else {
        return;
    };
    let root = std::env::temp_dir().join("Peekaboo").join("drag out");
    // Only the latest drag's file is kept. The PDF writer never replaces a
    // file, so each drag gets its own folder.
    let _ = std::fs::remove_dir_all(&root);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let folder = root.join(format!("{}-{stamp}", std::process::id()));
    let stem = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let output = folder.join(format!("{stem} page {}.pdf", page + 1));
    let request = Request {
        generation: s.generation,
        path,
        page,
        delta: 0,
        width: 1,
        height: 1,
        sessions: s.sessions.clone(),
        sources: s.opened_sources(),
    };
    let sent = std::fs::create_dir_all(&folder).is_ok() && s.send(Job::Save(request, output.clone(), SaveKind::DragOut));
    if let Some(drag) = s.organize.drag.as_mut() {
        drag.out = if sent { Out::Extracting(output) } else { Out::Failed };
    }
}

/// The document worker finished a page for a drag out.
pub(super) fn extracted(s: &mut State, output: PathBuf, result: Result<(), String>) {
    let waiting = s.organize.drag.as_mut().filter(|d| d.out == Out::Extracting(output.clone()));
    match (waiting, result) {
        (Some(drag), Ok(())) => drag.out = Out::Ready(output),
        (Some(drag), Err(error)) => {
            drag.out = Out::Failed;
            s.status = error;
        }
        (None, _) => {
            let _ = std::fs::remove_file(&output);
        }
    }
}

/// Work that opens dialogs or a modal drag loop, so it runs after tick
/// releases the state: a file drop, and a page ready to drag out.
pub(super) unsafe fn after_tick(hwnd: HWND) {
    if let Some((paths, at)) = with_state(|s| s.organize.dropped.take()).flatten() {
        super::drop::finish(hwnd, paths, at);
    }
    let ready = with_state(|s| {
        let drag = s.organize.drag.as_ref()?;
        let Out::Ready(path) = &drag.out else {
            return None;
        };
        let outside = !s.layout().sidebar.is_some_and(|r| r.contains(drag.at.0, drag.at.1));
        outside.then(|| path.clone())
    })
    .flatten();
    if let Some(path) = ready {
        drag_out(hwnd, path);
    }
}

unsafe fn drag_out(hwnd: HWND, path: PathBuf) {
    with_state(|s| {
        s.organize.drag = None;
        s.pressed = None;
        s.status = s.subtitle();
    });
    let _ = ReleaseCapture();
    invalidate(hwnd);
    DRAGGING_OUT.with(|d| d.set(true));
    let result = crate::integration::drag_files(hwnd, &[path]);
    DRAGGING_OUT.with(|d| d.set(false));
    if let Err(error) = result {
        with_state(|s| s.status = error);
    }
    invalidate(hwnd);
}

/// Scrolls the thumbnails while a drag rests near the list's top or bottom.
pub(super) fn autoscroll(s: &mut State) -> bool {
    let (x, y) = match &s.organize.drag {
        Some(d) if d.active => d.at,
        Some(_) => return false,
        None => match s.organize.drop_over {
            Some(at) => at,
            None => return false,
        },
    };
    if !thumbnails(s) {
        return false;
    }
    let layout = s.layout();
    let (Some(side), panel) = (layout.sidebar, layout.sidebar_panel) else {
        return false;
    };
    if x < side.x0 || x >= side.x1 {
        return false;
    }
    let zone = EDGE * s.scale;
    let push = if y < panel.y0 + zone {
        (y - panel.y0 - zone) / zone
    } else if y > panel.y1 - zone {
        (y - panel.y1 + zone) / zone
    } else {
        return false;
    };
    let max = (layout.sidebar_content - panel.height()).max(0.0);
    let before = s.sidebar_scroll[0];
    s.sidebar_scroll[0] = (before + push.clamp(-1.0, 1.0) * SPEED * s.scale).clamp(0.0, max);
    s.sidebar_scroll[0] != before
}

/// The insertion line, in client y, for a thumbnail drag or a file drop.
fn line(s: &State, layout: &Layout) -> Option<f32> {
    if !thumbnails(s) {
        return None;
    }
    let (x, y) = match &s.organize.drag {
        Some(d) if d.active => d.at,
        Some(_) => return None,
        None => s.organize.drop_over?,
    };
    if !layout.sidebar?.contains(x, y) {
        return None;
    }
    let gap = insertion(s, layout, y);
    if let Some(d) = &s.organize.drag {
        move_target(d.page, gap)?;
    }
    let rows = rows(s, layout);
    let inset = 2.0 * s.scale;
    rows.iter()
        .find(|(i, _)| *i as u32 == gap)
        .map(|(_, r)| r.y0 + inset)
        .or_else(|| rows.iter().find(|(i, _)| *i as u32 + 1 == gap).map(|(_, r)| r.y1 - inset))
}

pub(super) fn paint(p: &Painter, s: &State, layout: &Layout) {
    let Some(y) = line(s, layout) else {
        return;
    };
    let (scale, panel) = (s.scale, layout.sidebar_panel);
    let bar = Rect { x0: panel.x0 + 8.0 * scale, y0: y - 1.5 * scale, x1: panel.x1 - 8.0 * scale, y1: y + 1.5 * scale };
    p.push_clip(panel);
    p.fill_round(bar, 1.5 * scale, s.theme.accent);
    p.pop_clip();
}

unsafe fn move_page(hwnd: HWND, from: u32, to: u32) {
    actions::edit(hwnd, |s, edits| {
        edits.pdf.push(PdfEdit::Move { from, to });
        s.page = to;
        if matches!(s.focus, Some(WidgetId::SidebarItem(_))) {
            s.focus = Some(WidgetId::SidebarItem(to as usize));
        }
        sidebar::follow_page(s);
    });
}

/// Moves the current page one place up or down.
pub(super) unsafe fn step(hwnd: HWND, request: &Request, count: u32, down: bool) {
    let from = request.page;
    let to = if down { from + 1 } else { from.wrapping_sub(1) };
    if to < count {
        move_page(hwnd, from, to);
    }
}

/// Reads files to insert as pages at `gap`, in the order given. The recipe
/// holds the bytes, so later changes to the files do not change it.
fn page_inserts(paths: &[PathBuf], gap: u32) -> (Vec<PdfEdit>, Vec<String>) {
    let mut inserts = Vec::new();
    let mut skipped = Vec::new();
    for path in paths {
        let name = file_name(path);
        match can_insert(path).then(|| std::fs::read(path).ok()).flatten() {
            Some(bytes) if is_pdf(path) => inserts.push(PdfEdit::InsertPdf { at: gap, name, bytes: bytes.into() }),
            Some(bytes) => inserts.push(PdfEdit::InsertImage { at: gap, name, bytes: bytes.into() }),
            None => skipped.push(name),
        }
    }
    // Each insert at the same gap goes in front of the ones before it.
    inserts.reverse();
    (inserts, skipped)
}

unsafe fn report(hwnd: HWND, skipped: &[String]) {
    if !skipped.is_empty() {
        let message = format!("Only PDFs and images that Peekaboo can read become pages. Not added: {}", skipped.join(", "));
        sheet::alert(hwnd, "Some files were not added", &message);
    }
}

/// Asks for images and inserts them as pages after the current page.
pub(super) unsafe fn insert_images(hwnd: HWND, request: &Request) {
    let Some(paths) = choose_many(hwnd, true) else {
        return;
    };
    let gap = request.page + 1;
    let (inserts, skipped) = page_inserts(&paths, gap);
    if !inserts.is_empty() {
        actions::edit_same_page(hwnd, request, |s, edits| {
            edits.pdf.extend(inserts);
            s.page = gap;
        });
    }
    report(hwnd, &skipped);
}

/// Inserts dropped PDFs and images as pages at `gap`.
pub(super) unsafe fn insert_files(hwnd: HWND, paths: &[PathBuf], gap: u32) {
    let (inserts, skipped) = page_inserts(paths, gap);
    if !inserts.is_empty() {
        actions::edit(hwnd, |s, edits| {
            edits.pdf.extend(inserts);
            s.page = gap;
        });
    }
    report(hwnd, &skipped);
}
