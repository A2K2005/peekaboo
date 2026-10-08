//! Marks made with the markup tools: the style for new marks, and the
//! selected mark. A selected mark moves, resizes, restyles, and deletes by
//! replacing its step in the file's recipe, so the recipe stays the one
//! record of edits.
use super::{
    actions,
    app::{invalidate, schedule, with_state, State},
    commands::{Command, MenuItem, Pick, Preview},
    document::{self, Phase, PointerEvent, PointerKind},
    render::Painter,
    theme::Rgba,
    widgets::{Rect, WidgetId},
    worker::is_pdf,
};
use crate::model::{outline, AnnotationKind, ImageEdit, MarkFont, MarkStyle, NormRect, PdfEdit};
use std::{
    path::{Path, PathBuf},
    time::Instant,
};
use windows::Win32::{Foundation::HWND, UI::Input::KeyboardAndMouse::*};

const COLORS: [(&str, u32); 9] = [
    ("Black", 0x1A1A1AFF),
    ("White", 0xFFFFFFFF),
    ("Red", 0xE81123FF),
    ("Orange", 0xF7630CFF),
    ("Yellow", 0xFFB900FF),
    ("Green", 0x10893EFF),
    ("Blue", 0x0078D4FF),
    ("Purple", 0x881798FF),
    ("Gray", 0x7A7574FF),
];
const WIDTHS: [f32; 6] = [1.0, 2.0, 3.0, 5.0, 8.0, 12.0];
const SIZES: [f32; 8] = [9.0, 12.0, 14.0, 18.0, 24.0, 36.0, 48.0, 72.0];
const FONTS: [(&str, MarkFont); 3] = [("Sans serif", MarkFont::Sans), ("Serif", MarkFont::Serif), ("Monospace", MarkFont::Mono)];
/// Text style menu choices: fonts, then sizes from here, then colors.
const SIZE_BASE: usize = 100;
const COLOR_BASE: usize = 200;

#[derive(Default)]
pub(super) struct Marks {
    /// The look of the next mark.
    pub(super) style: MarkStyle,
    selected: Option<Selected>,
    drag: Option<Drag>,
    /// Recipes from before each in-place change (move, resize, restyle,
    /// delete). Undo pops the last step, which would remove a changed mark
    /// instead of changing it back, so it restores these first.
    restore: Vec<Restore>,
}

struct Restore {
    path: PathBuf,
    /// Recipe lengths right after the change; the restore applies only
    /// while the recipe is back at that point.
    lengths: (usize, usize),
    image: Vec<ImageEdit>,
    pdf: Vec<PdfEdit>,
}

impl Marks {
    pub(super) fn deselect(&mut self) {
        self.selected = None;
        self.drag = None;
    }

    /// Undoes the last in-place change to `path`'s recipe, if the recipe is
    /// at the point right after it. Returns false when Undo should pop a step.
    pub(super) fn undo(&mut self, path: &Path, edits: &mut super::worker::Edits) -> bool {
        let lengths = (edits.image.len(), edits.pdf.len());
        let Some(at) = self.restore.iter().rposition(|r| r.path == path) else {
            return false;
        };
        if self.restore[at].lengths != lengths {
            return false;
        }
        let restore = self.restore.remove(at);
        edits.image = restore.image;
        edits.pdf = restore.pdf;
        self.deselect();
        true
    }

    pub(super) fn forget(&mut self, path: &Path) {
        self.restore.retain(|r| r.path != path);
    }
}

/// One mark in a recipe, read from an `Annotate` or a `Mark` step.
#[derive(Clone, Debug, PartialEq)]
struct Mark {
    page: u32,
    kind: AnnotationKind,
    points: Vec<[f32; 2]>,
    text: String,
    style: MarkStyle,
}

struct Selected {
    path: PathBuf,
    index: usize,
    mark: Mark,
}

struct Drag {
    /// The corner being dragged, clockwise from the top left. None moves the mark.
    corner: Option<usize>,
    start: (f32, f32),
    /// The page or image size on screen, in pixels.
    page: [f32; 2],
    preview: Vec<[f32; 2]>,
}

fn read_pdf(edit: &PdfEdit) -> Option<Mark> {
    match edit {
        PdfEdit::Annotate { page, kind, points, text } => {
            Some(Mark { page: *page, kind: *kind, points: points.clone(), text: text.clone(), style: MarkStyle::default() })
        }
        PdfEdit::Mark { page, kind, points, text, style } => {
            Some(Mark { page: *page, kind: *kind, points: points.clone(), text: text.clone(), style: *style })
        }
        _ => None,
    }
}

fn read_image(edit: &ImageEdit) -> Option<Mark> {
    match edit {
        ImageEdit::Annotate { kind, points, text } => {
            Some(Mark { page: 0, kind: *kind, points: points.clone(), text: text.clone(), style: MarkStyle::default() })
        }
        ImageEdit::Mark { kind, points, text, style } => {
            Some(Mark { page: 0, kind: *kind, points: points.clone(), text: text.clone(), style: *style })
        }
        _ => None,
    }
}

fn step(state: &State, path: &Path, index: usize) -> Option<Mark> {
    let edits = state.sessions.get(path)?;
    if is_pdf(path) {
        read_pdf(edits.pdf.get(index)?)
    } else {
        read_image(edits.image.get(index)?)
    }
}

/// Text markup belongs to its text, so it does not move.
fn movable(kind: AnnotationKind) -> bool {
    !matches!(kind, AnnotationKind::Highlight | AnnotationKind::Underline | AnnotationKind::Strikeout)
}

/// The marks on `page` that can still move, newest first. The search stops
/// at a rotate, crop, or page change, because earlier marks no longer line
/// up with the page on screen, and at an edit that names annotations by
/// number, because removing an earlier mark would renumber them.
fn marks_on(state: &State, path: &Path, page: u32) -> Vec<(usize, Mark)> {
    let Some(edits) = state.sessions.get(path) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    if is_pdf(path) {
        for (index, edit) in edits.pdf.iter().enumerate().rev() {
            match edit {
                PdfEdit::RotateRight { page: p }
                | PdfEdit::Crop { page: p, .. }
                | PdfEdit::DeleteAnnotation { page: p, .. }
                | PdfEdit::SetAnnotationText { page: p, .. }
                    if *p == page =>
                {
                    break
                }
                PdfEdit::Delete { .. } | PdfEdit::Move { .. } | PdfEdit::InsertBlank { .. } | PdfEdit::InsertImage { .. } | PdfEdit::InsertPdf { .. } => {
                    break
                }
                _ => found.extend(read_pdf(edit).filter(|m| m.page == page && movable(m.kind)).map(|m| (index, m))),
            }
        }
    } else {
        for (index, edit) in edits.image.iter().enumerate().rev() {
            match edit {
                ImageEdit::RotateRight | ImageEdit::FlipHorizontal | ImageEdit::FlipVertical | ImageEdit::Crop { .. } => break,
                _ => found.extend(read_image(edit).filter(|m| movable(m.kind)).map(|m| (index, m))),
            }
        }
    }
    found
}

fn bounds(points: &[[f32; 2]]) -> NormRect {
    points.iter().fold([1.0f32, 1.0, 0.0, 0.0], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])
}

/// A normalized rectangle on the page at `page` (screen pixels).
fn on_screen(page: Rect, r: NormRect) -> Rect {
    Rect {
        x0: page.x0 + r[0] * page.width(),
        y0: page.y0 + r[1] * page.height(),
        x1: page.x0 + r[2] * page.width(),
        y1: page.y0 + r[3] * page.height(),
    }
}

fn corners(r: Rect) -> [(f32, f32); 4] {
    [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)]
}

/// The selected mark while it is still the same step of the open file.
fn live(state: &State) -> Option<&Selected> {
    state
        .marks
        .selected
        .as_ref()
        .filter(|s| state.path.as_ref() == Some(&s.path) && step(state, &s.path, s.index).as_ref() == Some(&s.mark))
}

/// Selects the last step of the file's recipe when it is a mark that moves.
pub(super) fn select_last(state: &mut State, path: &Path) {
    let count = state.sessions.get(path).map_or(0, |e| if is_pdf(path) { e.pdf.len() } else { e.image.len() });
    state.marks.drag = None;
    state.marks.selected = count.checked_sub(1).and_then(|index| {
        let mark = step(state, path, index).filter(|m| movable(m.kind))?;
        Some(Selected { path: path.to_path_buf(), index, mark })
    });
}

/// Adds a mark in the current style to the file's recipe. Shapes and text
/// stay selected, so they can move, resize, and restyle at once.
pub(super) unsafe fn add(hwnd: HWND, state: &mut State, path: &Path, page: u32, kind: AnnotationKind, points: Vec<[f32; 2]>, text: String) {
    let style = state.marks.style;
    let edits = state.sessions.entry(path.to_path_buf()).or_default();
    if is_pdf(path) {
        edits.pdf.push(PdfEdit::Mark { page, kind, points, text, style });
    } else {
        edits.image.push(ImageEdit::Mark { kind, points, text, style });
    }
    edits.dirty = true;
    state.edited_for_save(&path.to_path_buf(), Instant::now());
    if kind == AnnotationKind::Ink {
        state.marks.deselect();
    } else {
        select_last(state, path);
    }
    schedule(hwnd, state, 0);
}

/// Replaces the selected mark's step with `mark` and keeps it selected.
unsafe fn replace(hwnd: HWND, state: &mut State, mark: Mark) {
    let Some(selected) = state.marks.selected.take() else {
        return;
    };
    let edits = state.sessions.entry(selected.path.clone()).or_default();
    let before = (edits.image.clone(), edits.pdf.clone());
    let Mark { page, kind, points, text, style } = mark.clone();
    if is_pdf(&selected.path) {
        if let Some(edit) = edits.pdf.get_mut(selected.index) {
            *edit = PdfEdit::Mark { page, kind, points, text, style };
        }
    } else if let Some(edit) = edits.image.get_mut(selected.index) {
        *edit = ImageEdit::Mark { kind, points, text, style };
    }
    edits.dirty = true;
    let lengths = (edits.image.len(), edits.pdf.len());
    state.marks.restore.push(Restore { path: selected.path.clone(), lengths, image: before.0, pdf: before.1 });
    state.edited_for_save(&selected.path, Instant::now());
    state.marks.selected = Some(Selected { mark, ..selected });
    schedule(hwnd, state, 0);
}

unsafe fn remove(hwnd: HWND, state: &mut State) {
    let Some(selected) = state.marks.selected.take() else {
        return;
    };
    let edits = state.sessions.entry(selected.path.clone()).or_default();
    let before = (edits.image.clone(), edits.pdf.clone());
    if is_pdf(&selected.path) {
        edits.pdf.remove(selected.index);
    } else {
        edits.image.remove(selected.index);
    }
    edits.dirty = true;
    let lengths = (edits.image.len(), edits.pdf.len());
    state.marks.restore.push(Restore { path: selected.path.clone(), lengths, image: before.0, pdf: before.1 });
    state.edited_for_save(&selected.path, Instant::now());
    schedule(hwnd, state, 0);
}

/// The points after dragging by `delta` (page fractions): all of them, or,
/// from a corner, scaled from the opposite corner. The mark stays on the page.
fn moved(points: &[[f32; 2]], corner: Option<usize>, delta: [f32; 2]) -> Vec<[f32; 2]> {
    let b = bounds(points);
    let Some(corner) = corner else {
        let dx = delta[0].clamp(-b[0], 1.0 - b[2]);
        let dy = delta[1].clamp(-b[1], 1.0 - b[3]);
        return points.iter().map(|p| [p[0] + dx, p[1] + dy]).collect();
    };
    let (x, y) = ([b[0], b[2], b[2], b[0]][corner], [b[1], b[1], b[3], b[3]][corner]);
    let (ax, ay) = ([b[2], b[0], b[0], b[2]][corner], [b[3], b[3], b[1], b[1]][corner]);
    // A flat side (a level line) keeps its size; nothing shrinks to nothing.
    let scale = |from: f32, anchor: f32, d: f32| {
        if (from - anchor).abs() < 1e-4 {
            return 1.0;
        }
        let s = ((from + d).clamp(0.0, 1.0) - anchor) / (from - anchor);
        s.abs().max(0.05).copysign(s)
    };
    let (sx, sy) = (scale(x, ax, delta[0]), scale(y, ay, delta[1]));
    points.iter().map(|p| [(ax + (p[0] - ax) * sx).clamp(0.0, 1.0), (ay + (p[1] - ay) * sy).clamp(0.0, 1.0)]).collect()
}

/// Pointer input for marks: the selected mark's corners resize it and its
/// body moves it; with no tool on, a press on a mark selects it. Returns
/// None when the press is for something else.
pub(super) unsafe fn pointer(hwnd: HWND, state: &mut State, e: &PointerEvent) -> Option<bool> {
    match e.phase {
        Phase::Down => {
            let reach = 8.0 * state.scale;
            let busy = state.crop || state.zoom_select || state.signature.is_some();
            let grabbed = live(state).filter(|_| !busy).and_then(|s| {
                let r = document::text_rect(state, s.mark.page, bounds(&s.mark.points))?;
                let corner = corners(r).iter().position(|c| (c.0 - e.x).abs() <= reach && (c.1 - e.y).abs() <= reach);
                (corner.is_some() || r.inset(-reach).contains(e.x, e.y)).then_some(corner)
            });
            let corner = match grabbed {
                Some(corner) => corner,
                None => {
                    if state.marks.selected.is_some() {
                        // The handles go away even when nothing else repaints.
                        invalidate(hwnd);
                    }
                    state.marks.deselect();
                    if busy || state.markup.is_some() {
                        return None;
                    }
                    let (page, at, size) = document::text_point(state, e.x, e.y)?;
                    let path = state.path.clone()?;
                    let slop = [reach / size[0].max(1.0), reach / size[1].max(1.0)];
                    let (index, mark) = marks_on(state, &path, page).into_iter().find(|(_, m)| {
                        let b = bounds(&m.points);
                        at[0] >= b[0] - slop[0] && at[0] <= b[2] + slop[0] && at[1] >= b[1] - slop[1] && at[1] <= b[3] + slop[1]
                    })?;
                    state.marks.selected = Some(Selected { path, index, mark });
                    None
                }
            };
            let selected = state.marks.selected.as_ref()?;
            let page = document::text_rect(state, selected.mark.page, [0.0, 0.0, 1.0, 1.0])?;
            let preview = selected.mark.points.clone();
            state.marks.drag = Some(Drag { corner, start: (e.x, e.y), page: [page.width().max(1.0), page.height().max(1.0)], preview });
            state.drag = Some((e.x, e.y));
            if e.kind == PointerKind::Mouse {
                SetCapture(hwnd);
            }
            Some(true)
        }
        Phase::Move => {
            let marks = &mut state.marks;
            let (drag, selected) = (marks.drag.as_mut()?, marks.selected.as_ref()?);
            let delta = [(e.x - drag.start.0) / drag.page[0], (e.y - drag.start.1) / drag.page[1]];
            drag.preview = moved(&selected.mark.points, drag.corner, delta);
            Some(true)
        }
        Phase::Up | Phase::Cancel => {
            let drag = state.marks.drag.take()?;
            state.drag = None;
            let _ = ReleaseCapture();
            let mark = live(state).map(|s| s.mark.clone()).filter(|m| e.phase == Phase::Up && m.points != drag.preview);
            if let Some(mark) = mark {
                replace(hwnd, state, Mark { points: drag.preview, ..mark });
            }
            Some(true)
        }
    }
}

/// Delete removes the selected mark; Escape deselects it.
pub(super) unsafe fn key(hwnd: HWND, state: &mut State, key: VIRTUAL_KEY) -> bool {
    if live(state).is_none() {
        return false;
    }
    match key {
        VK_DELETE | VK_BACK => remove(hwnd, state),
        VK_ESCAPE => state.marks.deselect(),
        _ => return false,
    }
    true
}

fn colors(current: u32, none: bool, base: usize) -> Vec<MenuItem> {
    let off = |c: u32| c & 0xFF == 0;
    none.then_some(("None", 0))
        .into_iter()
        .chain(COLORS)
        .enumerate()
        .map(|(index, (name, color))| MenuItem {
            preview: Some(Preview::Color(color)),
            checked: Some(color == current || (off(color) && off(current))),
            ..MenuItem::choice(name, base + index)
        })
        .collect()
}

fn color_at(index: usize, none: bool) -> Option<u32> {
    match (none, index) {
        (true, 0) => Some(0),
        (true, i) => COLORS.get(i - 1).map(|c| c.1),
        (false, i) => COLORS.get(i).map(|c| c.1),
    }
}

/// Line width, border color, fill color, and text style menus. A choice
/// applies to the selected mark and to the marks drawn next.
pub(super) unsafe fn style_menu(hwnd: HWND, command: Command, keyboard: bool) {
    let Some((style, pdf)) = with_state(|s| (live(s).map_or(s.marks.style, |selected| selected.mark.style), s.is_pdf())) else {
        return;
    };
    let items = match command {
        Command::LineWidthMenu => WIDTHS
            .iter()
            .enumerate()
            .map(|(index, width)| MenuItem {
                preview: Some(Preview::Width(*width)),
                checked: Some(style.width == *width),
                ..MenuItem::choice(&format!("{width} pt"), index)
            })
            .collect(),
        Command::BorderColorMenu => colors(style.stroke, true, 0),
        Command::FillColorMenu => colors(style.fill, true, 0),
        _ => {
            // PDF text is drawn in Helvetica, so only images offer fonts.
            let mut menu: Vec<MenuItem> = if pdf {
                Vec::new()
            } else {
                FONTS
                    .iter()
                    .enumerate()
                    .map(|(index, (name, font))| MenuItem { checked: Some(style.font == *font), ..MenuItem::choice(name, index) })
                    .chain(Some(MenuItem::separator()))
                    .collect()
            };
            menu.extend(SIZES.iter().enumerate().map(|(index, size)| MenuItem {
                checked: Some(style.size == *size),
                ..MenuItem::choice(&format!("{size} pt"), SIZE_BASE + index)
            }));
            menu.push(MenuItem::separator());
            menu.push(MenuItem::submenu("Text color", colors(style.text, false, COLOR_BASE)));
            menu
        }
    };
    let Some(Pick::Index(index)) = actions::popup(hwnd, items, Some(WidgetId::Command(command)), None, keyboard) else {
        return;
    };
    let mut next = style;
    match command {
        Command::LineWidthMenu => next.width = WIDTHS.get(index).copied().unwrap_or(style.width),
        Command::BorderColorMenu => next.stroke = color_at(index, true).unwrap_or(style.stroke),
        Command::FillColorMenu => next.fill = color_at(index, true).unwrap_or(style.fill),
        _ if index >= COLOR_BASE => next.text = color_at(index - COLOR_BASE, false).unwrap_or(style.text),
        _ if index >= SIZE_BASE => next.size = SIZES.get(index - SIZE_BASE).copied().unwrap_or(style.size),
        _ => next.font = FONTS.get(index).map_or(style.font, |f| f.1),
    }
    with_state(|s| {
        s.marks.style = next;
        let Some(mut mark) = live(s).map(|selected| Mark { style: next, ..selected.mark.clone() }) else {
            return;
        };
        // A text box grows or shrinks with its text from the top-left corner.
        if mark.kind == AnnotationKind::Text && next.size != style.size {
            let (b, k) = (bounds(&mark.points), next.size / style.size);
            mark.points = mark.points.iter().map(|p| [(b[0] + (p[0] - b[0]) * k).min(1.0), (b[1] + (p[1] - b[1]) * k).min(1.0)]).collect();
        }
        replace(hwnd, s, mark);
    });
    invalidate(hwnd);
}

/// The outline of a mark of `kind` through `points` (page fractions) on the
/// page at `page` (screen pixels), drawn while it is placed or dragged.
pub(super) fn shape(p: &Painter, kind: AnnotationKind, points: &[[f32; 2]], page: Rect, color: Rgba, width: f32) {
    use AnnotationKind::*;
    let at = |q: [f32; 2]| (page.x0 + q[0] * page.width(), page.y0 + q[1] * page.height());
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return;
    };
    let (a, b) = (at(*first), at(*last));
    let r = Rect { x0: a.0.min(b.0), y0: a.1.min(b.1), x1: a.0.max(b.0), y1: a.1.max(b.1) };
    match kind {
        Ink => {
            for pair in points.windows(2) {
                p.line(at(pair[0]), at(pair[1]), color, width);
            }
        }
        Line | Arrow => {
            p.line(a, b, color, width);
            if kind == Arrow {
                let angle = (b.1 - a.1).atan2(b.0 - a.0);
                for offset in [-0.5f32, 0.5] {
                    let tip = (b.0 - 6.0 * width * (angle + offset).cos(), b.1 - 6.0 * width * (angle + offset).sin());
                    p.line(b, tip, color, width);
                }
            }
        }
        Ellipse => p.stroke_ellipse(r, color, width),
        Loupe => {
            let d = r.width().min(r.height());
            p.stroke_ellipse(Rect::new((r.x0 + r.x1 - d) / 2.0, (r.y0 + r.y1 - d) / 2.0, d, d), color, width);
        }
        RoundedRectangle | Bubble | Star | Polygon => {
            let shape = outline(kind, *first, *last, page.width() / page.height().max(1.0)).unwrap_or_default();
            for (index, q) in shape.iter().enumerate() {
                p.line(at(*q), at(shape[(index + 1) % shape.len()]), color, width);
            }
        }
        _ => p.stroke_round(r, 0.0, color, width),
    }
}

/// Draws the selected mark's corner handles, and its outline while dragged.
pub(super) fn paint(p: &Painter, state: &State) {
    let Some(selected) = live(state) else {
        return;
    };
    let Some(page) = document::text_rect(state, selected.mark.page, [0.0, 0.0, 1.0, 1.0]) else {
        return;
    };
    let (s, accent) = (state.scale, state.theme.accent);
    let points = state.marks.drag.as_ref().map_or(&selected.mark.points, |d| &d.preview);
    if state.marks.drag.is_some() {
        shape(p, selected.mark.kind, points, page, accent, 1.5 * s);
    }
    for (x, y) in corners(on_screen(page, bounds(points))) {
        let handle = Rect::new(x - 5.0 * s, y - 5.0 * s, 10.0 * s, 10.0 * s);
        p.fill_round(handle, 5.0 * s, Rgba(1.0, 1.0, 1.0, 1.0));
        p.fill_round(handle.inset(1.5 * s), 3.5 * s, accent);
    }
}

/// The current color under a border or fill color button, as Windows color
/// buttons show it. A line through the strip means none.
pub(super) fn paint_button(p: &Painter, state: &State, command: Command, r: Rect) {
    let style = live(state).map_or(state.marks.style, |selected| selected.mark.style);
    let color = match command {
        Command::BorderColorMenu => style.stroke,
        Command::FillColorMenu => style.fill,
        _ => return,
    };
    let s = state.scale;
    let strip = Rect::new((r.x0 + r.x1) / 2.0 - 8.0 * s, r.y1 - 7.0 * s, 16.0 * s, 3.0 * s);
    let [red, green, blue, alpha] = crate::model::rgba(color);
    if alpha > 0.0 {
        p.fill(strip, Rgba(red, green, blue, 1.0));
        p.stroke_round(strip, 0.0, state.theme.control_border, s.max(1.0).floor());
    } else {
        p.stroke_round(strip, 0.0, state.theme.text_secondary, s.max(1.0).floor());
        p.line((strip.x0, strip.y1), (strip.x1, strip.y0), Rgba::hex(0xE81123), s.max(1.0).floor());
    }
}
