//! The document view: PDF pages as cached tiles, or one image; scrolling,
//! zoom, and pointer input on it. Mouse, pen, and touch arrive as one
//! `PointerEvent` type from WM_POINTER, so pen input (with pressure) can go
//! straight to ink.
//!
//! PDF pages render in 512-pixel tiles at the current zoom. A page's
//! thumbnail, stretched, stands in for tiles that have not arrived, so no
//! page area stays blank while scrolling. Tiles render at `render_scale`;
//! while the zoom changes (pinch, Ctrl+wheel, resize), the old tiles are
//! drawn scaled, and new tiles are asked for once the zoom has been still
//! for `SETTLE`.
use super::{
    app::{schedule, State},
    cache::Lru,
    render::Painter,
    theme::Rgba,
    view::{self, Layout, Position, ViewMode, Zoom},
    widgets::{self, Rect},
    worker::{doc_id, Item, Key, Note, Work},
};
use crate::model::{AnnotationKind, ImageEdit, OutlineItem, PdfEdit};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::HWND,
    Graphics::Direct2D::ID2D1Bitmap,
    UI::Input::KeyboardAndMouse::*,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PointerKind {
    Mouse,
    Pen,
    Touch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Down,
    Move,
    Up,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PointerEvent {
    pub(super) id: u32,
    pub(super) kind: PointerKind,
    pub(super) phase: Phase,
    /// Client pixels.
    pub(super) x: f32,
    pub(super) y: f32,
    /// Pen pressure from 0 to 1 (POINTER_PEN_INFO.pressure / 1024). None for
    /// mouse and touch.
    pub(super) pressure: Option<f32>,
    /// The primary button, pen tip, or finger is down.
    pub(super) contact: bool,
    pub(super) eraser: bool,
}

/// Two-finger gesture state, captured when the second finger lands.
/// `scale` is the display scale then; `origin` is the PDF view origin or
/// the image pan.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Pinch {
    ids: [u32; 2],
    start: [(f32, f32); 2],
    scale: f32,
    origin: (f32, f32),
}

/// Zoom ratio and pan offset from two finger positions: the distance ratio
/// and the movement of the midpoint.
pub(super) fn pinch_transform(start: [(f32, f32); 2], now: [(f32, f32); 2]) -> (f32, (f32, f32)) {
    let distance = |p: [(f32, f32); 2]| ((p[0].0 - p[1].0).powi(2) + (p[0].1 - p[1].1).powi(2)).sqrt();
    let mid = |p: [(f32, f32); 2]| ((p[0].0 + p[1].0) / 2.0, (p[0].1 + p[1].1) / 2.0);
    let ratio = if distance(start) < 1.0 { 1.0 } else { distance(now) / distance(start) };
    let (a, b) = (mid(start), mid(now));
    (ratio, (b.0 - a.0, b.1 - a.1))
}

/// Space between pages, in epx.
pub(super) const GAP: f32 = 12.0;
/// Arrow-key scroll step, in epx.
const LINE: f32 = 48.0;
/// One mouse wheel notch (120 units), in epx.
const NOTCH: f32 = 100.0;
const SCROLL_TIME: Duration = Duration::from_millis(150);
/// How long the zoom must stay still before tiles render at the new scale.
pub(super) const SETTLE: Duration = Duration::from_millis(120);
const WHITE: Rgba = Rgba(1.0, 1.0, 1.0, 1.0);

pub(super) struct Tile {
    pub(super) bitmap: ID2D1Bitmap,
    /// False after the document changed; still drawn until replaced.
    pub(super) fresh: bool,
}
pub(super) type Tiles = Lru<Key, Tile>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Scroll {
    from: f32,
    to: f32,
    start: Instant,
    scale: f32,
}

/// An open PDF as the view shows it.
pub(super) struct PdfView {
    pub(super) path: PathBuf,
    pub(super) doc: u64,
    /// The edits the page sizes and tiles were made with.
    pub(super) edits: Arc<Vec<PdfEdit>>,
    /// Page sizes in points.
    pub(super) sizes: Vec<[f32; 2]>,
    pub(super) position: Position,
    pub(super) scroll_x: f32,
    pub(super) render_scale: f32,
    /// The previous render scale; its tiles stand in until new ones arrive.
    pub(super) fallback_scale: f32,
    last_scale: f32,
    pub(super) scroll: Option<Scroll>,
    /// Bring this page into view after the next layout.
    pub(super) reveal: Option<u32>,
    pub(super) outline: Option<Result<Vec<OutlineItem>, String>>,
    /// Notes found so far, in page order, and which pages were read.
    pub(super) notes: Vec<Note>,
    pub(super) scanned: Vec<bool>,
    /// Work that failed; not asked for again until the document changes.
    pub(super) failed: HashSet<Key>,
}

impl PdfView {
    pub(super) fn new(path: PathBuf, sizes: Vec<[f32; 2]>, edits: Arc<Vec<PdfEdit>>, page: u32) -> Self {
        let page = page.min(sizes.len().saturating_sub(1) as u32);
        Self {
            doc: doc_id(&path),
            path,
            edits,
            scanned: vec![false; sizes.len()],
            sizes,
            position: Position { page, offset: 0.0 },
            scroll_x: 0.0,
            render_scale: 0.0,
            fallback_scale: 0.0,
            last_scale: 0.0,
            scroll: None,
            reveal: None,
            outline: None,
            notes: Vec::new(),
            failed: HashSet::new(),
        }
    }
    /// The same file reopened: new edits mark every cached tile stale and
    /// reload the outline and notes.
    pub(super) fn update(&mut self, sizes: Vec<[f32; 2]>, edits: Arc<Vec<PdfEdit>>, tiles: &mut Tiles) -> bool {
        let changed = *self.edits != *edits;
        if changed {
            for (key, tile) in tiles.values_mut() {
                if key.doc == self.doc {
                    tile.fresh = false;
                }
            }
            self.outline = None;
            self.notes.clear();
            self.failed.clear();
        }
        if changed || sizes.len() != self.sizes.len() {
            self.scanned = vec![false; sizes.len()];
        }
        self.sizes = sizes;
        self.edits = edits;
        changed
    }
}

/// The PDF view's geometry for one frame. `left` and `top` are the content
/// coordinates at the view's top-left corner.
pub(super) struct Geometry {
    pub(super) doc: Rect,
    pub(super) scale: f32,
    pub(super) layout: Layout,
    pub(super) left: f32,
    pub(super) top: f32,
}

pub(super) fn gap(state: &State) -> f32 {
    (GAP * state.scale).round()
}

pub(super) fn geometry_in(state: &State, doc: Rect) -> Option<Geometry> {
    let v = state.pdf.as_ref()?;
    let gap = gap(state);
    let scale = view::pdf_scale(&v.sizes, state.view_mode, state.zoom, (doc.width(), doc.height()), gap, view::actual(state.scale));
    let layout = view::layout(&v.sizes, state.view_mode, state.page, scale, gap);
    let top = view::origin(layout.height, doc.height(), view::to_y(&layout, v.position, scale));
    let left = view::origin(layout.width, doc.width(), v.scroll_x);
    Some(Geometry { doc, scale, layout, left, top })
}

pub(super) fn geometry(state: &State) -> Option<Geometry> {
    geometry_in(state, state.layout().document)
}

/// The page under a client point, and its screen rectangle.
fn page_at(g: &Geometry, x: f32, y: f32) -> Option<(u32, Rect)> {
    g.layout.pages.iter().map(|(page, r)| (*page, screen(g, *r))).find(|(_, r)| r.contains(x, y))
}

fn screen(g: &Geometry, r: Rect) -> Rect {
    let x0 = (g.doc.x0 + r.x0 - g.left).round();
    let y0 = (g.doc.y0 + r.y0 - g.top).round();
    Rect { x0, y0, x1: x0 + r.width(), y1: y0 + r.height() }
}

/// Moves the view top to content `y` and makes the page in the middle of
/// the view the current page.
fn set_top(state: &mut State, g: &Geometry, y: f32) {
    let y = y.clamp(0.0, (g.layout.height - g.doc.height()).max(0.0));
    let current = view::current(&g.layout, y + g.doc.height() / 2.0);
    let Some(v) = state.pdf.as_mut() else {
        return;
    };
    v.position = view::from_y(&g.layout, y, g.scale);
    if let Some(page) = current {
        if page != state.page && !state.pending && state.view_mode != ViewMode::Single {
            state.page = page;
            state.status = state.subtitle();
            super::sidebar::follow_page(state);
        }
    }
}

/// Scrolls to content `y`, gliding when `smooth` and animations are on.
fn scroll_to(state: &mut State, g: &Geometry, y: f32, smooth: bool) {
    let to = y.clamp(0.0, (g.layout.height - g.doc.height()).max(0.0));
    let animate = smooth && state.animations;
    let Some(v) = state.pdf.as_mut() else {
        return;
    };
    if animate {
        v.scroll = Some(Scroll { from: g.top, to, start: Instant::now(), scale: g.scale });
    } else {
        v.scroll = None;
        set_top(state, g, to);
    }
}

/// Scrolls the PDF view by device pixels. A running glide extends.
pub(super) fn scroll_by(state: &mut State, dx: f32, dy: f32, smooth: bool) {
    let Some(g) = geometry(state) else {
        return;
    };
    let Some(v) = state.pdf.as_mut() else {
        return;
    };
    if dx != 0.0 {
        v.scroll_x = g.left + dx;
    }
    if dy != 0.0 {
        let base = v.scroll.filter(|s| s.scale == g.scale).map_or(g.top, |s| s.to);
        scroll_to(state, &g, base + dy, smooth);
    }
}

/// Shows `page` at the top of the view.
pub(super) fn go_to_page(state: &mut State, page: u32, smooth: bool) {
    let Some(count) = state.pdf.as_ref().map(|v| v.sizes.len() as u32) else {
        return;
    };
    let page = page.min(count.saturating_sub(1));
    if state.view_mode == ViewMode::Single {
        state.page = page;
        if let Some(v) = state.pdf.as_mut() {
            v.position = Position { page, offset: 0.0 };
            v.scroll = None;
        }
        state.status = state.subtitle();
        super::sidebar::follow_page(state);
        return;
    }
    let Some(g) = geometry(state) else {
        return;
    };
    if let Some((_, r)) = g.layout.pages.iter().find(|(p, _)| *p == page) {
        scroll_to(state, &g, r.y0 - gap(state), smooth);
        // The page asked for is current even when the end of the document
        // keeps it out of the middle of the view.
        if !state.pending {
            state.page = page;
            state.status = state.subtitle();
            super::sidebar::follow_page(state);
        }
    }
}

/// Previous or next page; a row at a time in two-page view.
pub(super) fn step_page(state: &mut State, delta: i32) -> bool {
    let Some(count) = state.pdf.as_ref().map(|v| v.sizes.len() as i64) else {
        return false;
    };
    let step = if state.view_mode == ViewMode::TwoPages { 2 } else { 1 };
    let row = if step == 2 { state.page as i64 / 2 * 2 } else { state.page as i64 };
    let page = row + delta as i64 * step;
    if page < 0 || page >= count {
        return false;
    }
    go_to_page(state, page as u32, true);
    true
}

/// Scroll keys on the PDF view. Left and Right stay with the Previous and
/// Next commands.
pub(super) fn key(state: &mut State, vk: u16, shift: bool) -> bool {
    if state.pdf.is_none() || state.sheet.is_some() {
        return false;
    }
    let Some(g) = geometry(state) else {
        return false;
    };
    let line = LINE * state.scale;
    let screenful = (g.doc.height() - line).max(line);
    let max = (g.layout.height - g.doc.height()).max(0.0);
    let single = state.view_mode == ViewMode::Single;
    let key = VIRTUAL_KEY(vk);
    match key {
        VK_UP => scroll_by(state, 0.0, -line, true),
        VK_DOWN => scroll_by(state, 0.0, line, true),
        VK_PRIOR | VK_NEXT | VK_SPACE => {
            let down = key == VK_NEXT || (key == VK_SPACE && !shift);
            let at_edge = if down { g.top >= max - 1.0 } else { g.top <= 1.0 };
            if single && at_edge {
                let last = state.pdf.as_ref().map_or(0, |v| v.sizes.len() as u32 - 1);
                let page = if down { (state.page + 1).min(last) } else { state.page.saturating_sub(1) };
                if page != state.page {
                    go_to_page(state, page, false);
                    if !down {
                        if let Some(v) = state.pdf.as_mut() {
                            // Past the end; drawing clamps it to the page bottom.
                            v.position.offset = 1e7;
                        }
                    }
                }
            } else {
                scroll_by(state, 0.0, if down { screenful } else { -screenful }, true);
            }
        }
        VK_HOME if single => go_to_page(state, 0, false),
        VK_END if single => go_to_page(state, u32::MAX, false),
        VK_HOME => scroll_to(state, &g, 0.0, true),
        VK_END => scroll_to(state, &g, max, true),
        _ => return false,
    }
    true
}

/// Mouse wheel and touchpad. Ctrl zooms around the pointer. Whole notches
/// glide; touchpad deltas move at once.
pub(super) fn wheel(state: &mut State, delta: f32, horizontal: bool, ctrl: bool, at: (f32, f32)) {
    if ctrl && !horizontal {
        zoom_by(state, 1.25f32.powf(delta / 120.0), Some(at));
        return;
    }
    if state.pdf.is_some() {
        let amount = -delta / 120.0 * NOTCH * state.scale;
        let smooth = delta.abs() >= 120.0 && delta % 120.0 == 0.0;
        if horizontal {
            scroll_by(state, -amount, 0.0, false);
        } else {
            scroll_by(state, 0.0, amount, smooth);
        }
    } else if horizontal {
        state.pan.0 -= delta;
    } else {
        state.pan.1 += delta;
    }
}

fn center(r: Rect) -> (f32, f32) {
    ((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0)
}

fn image_view(state: &State) -> Option<(Rect, (u32, u32))> {
    let frame = state.frame.as_ref()?;
    Some((state.layout().document, (frame.source_width, frame.source_height)))
}

/// The zoom as a fraction of actual size.
pub(super) fn ratio(state: &State) -> f32 {
    if let Some(g) = geometry(state) {
        g.scale / view::actual(state.scale)
    } else if let Some((doc, source)) = image_view(state) {
        view::image_scale(state.zoom, (doc.width(), doc.height()), source)
    } else {
        1.0
    }
}

/// Applies a zoom, keeping the content under `anchor` (client pixels; the
/// view center when None) in place.
pub(super) fn set_zoom(state: &mut State, zoom: Zoom, anchor: Option<(f32, f32)>) {
    if state.pdf.is_some() {
        let Some(old) = geometry(state) else {
            return;
        };
        let anchor = anchor.unwrap_or(center(old.doc));
        state.zoom = zoom;
        let Some(new) = geometry(state) else {
            return;
        };
        let local = (anchor.0 - old.doc.x0, anchor.1 - old.doc.y0);
        let (x, y) = view::anchor(&old.layout, (old.left, old.top), &new.layout, local);
        if let Some(v) = state.pdf.as_mut() {
            v.scroll_x = x;
            v.scroll = None;
        }
        set_top(state, &new, y);
        if zoom == Zoom::Fit {
            // Fit page shows the current page whole.
            go_to_page(state, state.page, false);
        }
    } else if let Some((doc, source)) = image_view(state) {
        let size = (doc.width(), doc.height());
        let old = view::image_scale(state.zoom, size, source);
        state.zoom = zoom;
        let new = view::image_scale(zoom, size, source);
        state.pan = match zoom {
            Zoom::Ratio(_) => view::image_pan(doc, state.pan, new / old.max(1e-6), anchor.unwrap_or(center(doc))),
            _ => (0.0, 0.0),
        };
        // Decode again at the new size once the zoom is still.
        state.due = Some(Instant::now() + SETTLE);
    } else {
        state.zoom = zoom;
    }
}

pub(super) fn zoom_by(state: &mut State, factor: f32, anchor: Option<(f32, f32)>) {
    let ratio = (ratio(state) * factor).clamp(view::MIN_RATIO, view::MAX_RATIO);
    set_zoom(state, Zoom::Ratio(ratio), anchor);
}

pub(super) fn zoom_step(state: &mut State, up: bool) {
    set_zoom(state, Zoom::Ratio(view::step(ratio(state), up)), None);
}

/// The decode box for the current image: the view at fit, or the shown
/// size at other zooms, at most 4096 pixels a side.
pub(super) fn image_box(state: &State, doc: Rect) -> (u32, u32) {
    let fit = ((doc.width() as u32).clamp(1, 4096), (doc.height() as u32).clamp(1, 4096));
    let same = state.displayed.as_ref().map(|d| &d.0) == state.path.as_ref();
    match (&state.frame, state.zoom) {
        (Some(frame), zoom) if same && zoom != Zoom::Fit => {
            let source = (frame.source_width, frame.source_height);
            let s = view::image_scale(zoom, (doc.width(), doc.height()), source);
            let side = |n: u32| ((n as f32 * s).ceil() as u32).clamp(1, 4096);
            (side(source.0), side(source.1))
        }
        _ => fit,
    }
}

/// Per-frame work before drawing: the first render scale, settling after
/// zoom changes, revealing a page, and the scroll glide. Returns true while
/// the glide runs, so the caller paints again.
pub(super) fn prepare(state: &mut State, doc: Rect) -> bool {
    let Some(g) = geometry_in(state, doc) else {
        return false;
    };
    let due = state.due.is_none();
    let gap = gap(state);
    let Some(v) = state.pdf.as_mut() else {
        return false;
    };
    if v.render_scale <= 0.0 {
        v.render_scale = g.scale;
        v.last_scale = g.scale;
        if v.position.offset == 0.0 {
            // Open with the gap above the first page in view.
            v.position.offset = -gap / g.scale;
        }
    }
    if g.scale != v.last_scale || (v.render_scale != g.scale && due) {
        v.last_scale = g.scale;
        state.due = Some(Instant::now() + SETTLE);
    }
    if let Some(page) = v.reveal.take() {
        let shown = view::visible(&g.layout, g.top, g.top + doc.height());
        if !g.layout.pages[shown].iter().any(|(p, _)| *p == page) {
            v.position = Position { page, offset: 0.0 };
            v.scroll = None;
            return false;
        }
    }
    let Some(glide) = v.scroll else {
        return false;
    };
    if glide.scale != g.scale {
        v.scroll = None;
        return false;
    }
    let t = (glide.start.elapsed().as_secs_f32() / SCROLL_TIME.as_secs_f32()).min(1.0);
    if t >= 1.0 {
        v.scroll = None;
    }
    set_top(state, &g, glide.from + (glide.to - glide.from) * super::app::ease(t));
    t < 1.0
}

/// Renders tiles at the current display scale. Returns true when it changed.
pub(super) fn settle(state: &mut State) -> bool {
    let Some(g) = geometry(state) else {
        return false;
    };
    let Some(v) = state.pdf.as_mut() else {
        return false;
    };
    if v.render_scale == g.scale {
        return false;
    }
    v.fallback_scale = v.render_scale;
    v.render_scale = g.scale;
    true
}

/// What one frame of the PDF view showed. The scroll benchmark reports it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Stats {
    /// Tiles at the render scale that overlap the view.
    pub(super) visible: u32,
    /// Of those, tiles not rendered yet.
    pub(super) missing: u32,
    /// Missing tiles with no thumbnail under them either.
    pub(super) blank: Vec<Key>,
    /// Pixel size of the first visible page at the render scale.
    pub(super) page_px: (u32, u32),
}

/// Draws the tiles of one page at `scale` that overlap `clip`.
#[allow(clippy::too_many_arguments)]
fn draw_tiles(
    p: &Painter,
    tiles: &mut Tiles,
    doc: u64,
    page: u32,
    size: [f32; 2],
    scale: f32,
    display: f32,
    dest: Rect,
    clip: Rect,
    mut stats: Option<(&mut Stats, bool)>,
) {
    let ratio = display / scale;
    let local = Rect {
        x0: (clip.x0.max(dest.x0) - dest.x0) / ratio,
        y0: (clip.y0.max(dest.y0) - dest.y0) / ratio,
        x1: (clip.x1.min(dest.x1) - dest.x0) / ratio,
        y1: (clip.y1.min(dest.y1) - dest.y0) / ratio,
    };
    for (col, row, [x, y, w, h]) in view::tiles(view::page_px(size, scale), local) {
        let key = Key { doc, work: Work::Tile { page, scale: scale.to_bits(), col, row } };
        let at = |n: u32, origin: f32| (origin + n as f32 * ratio).round();
        let r = Rect { x0: at(x, dest.x0), y0: at(y, dest.y0), x1: at(x + w, dest.x0), y1: at(y + h, dest.y0) };
        let tile = tiles.get(&key);
        if let Some(tile) = &tile {
            p.draw_bitmap(&tile.bitmap, r);
        }
        if let Some((stats, thumb)) = stats.as_mut() {
            stats.visible += 1;
            if tile.is_none() {
                stats.missing += 1;
                if !*thumb {
                    stats.blank.push(key);
                }
            }
        }
    }
}

fn paint_pdf(p: &Painter, state: &mut State, doc: Rect) -> bool {
    let Some(g) = geometry_in(state, doc) else {
        return false;
    };
    let (border, s) = (state.theme.border, state.scale);
    let Some(v) = state.pdf.as_ref() else {
        return false;
    };
    let render = if v.render_scale > 0.0 { v.render_scale } else { g.scale };
    let mut stats = Stats::default();
    p.push_clip(doc);
    for (page, rect) in &g.layout.pages[view::visible(&g.layout, g.top, g.top + doc.height())] {
        let dest = screen(&g, *rect);
        let size = v.sizes[*page as usize];
        p.fill(dest, WHITE);
        let thumb = state.cache.get(&Key { doc: v.doc, work: Work::Thumb { page: *page } });
        let has_thumb = thumb.is_some();
        if let Some(thumb) = thumb {
            p.draw_bitmap(&thumb.bitmap, dest);
        }
        if v.fallback_scale > 0.0 && v.fallback_scale != render {
            draw_tiles(p, &mut state.cache, v.doc, *page, size, v.fallback_scale, g.scale, dest, doc, None);
        }
        if stats.visible == 0 {
            stats.page_px = view::page_px(size, render);
        }
        draw_tiles(p, &mut state.cache, v.doc, *page, size, render, g.scale, dest, doc, Some((&mut stats, has_thumb)));
        let outline = Rect { x0: dest.x0 - s, y0: dest.y0 - s, x1: dest.x1 + s, y1: dest.y1 + s };
        p.stroke_round(outline, 0.0, border, s);
    }
    p.pop_clip();
    let complete = stats.visible > 0 && stats.missing == 0;
    state.stats = stats;
    complete
}

/// Draws the canvas, the PDF pages or the image, and in-progress markup.
/// Returns true when the view shows complete content: the image, or every
/// visible tile at full resolution. The launch benchmark needs real content.
pub(super) fn paint(p: &Painter, bitmap: Option<&ID2D1Bitmap>, state: &mut State, doc: Rect) -> bool {
    let theme = state.theme;
    p.fill(doc, theme.canvas);
    let drew = if state.pdf.is_some() {
        paint_pdf(p, state, doc)
    } else if let (Some(frame), Some(bitmap)) = (state.frame.as_ref(), bitmap) {
        let source = (frame.source_width, frame.source_height);
        let scale = view::image_scale(state.zoom, (doc.width(), doc.height()), source);
        let rect = view::image_rect(doc, source, scale, state.pan);
        state.image_rect = rect;
        p.push_clip(doc);
        p.draw_bitmap(bitmap, rect);
        let s = state.scale;
        let outline = Rect { x0: rect.x0 - s, y0: rect.y0 - s, x1: rect.x1 + s, y1: rect.y1 + s };
        p.stroke_round(outline, 0.0, theme.border, s);
        p.pop_clip();
        true
    } else {
        false
    };
    let s = state.scale;
    p.push_clip(doc);
    if state.markup == Some(AnnotationKind::Ink) && state.signature.is_none() {
        for pair in state.ink.windows(2) {
            p.line((pair[0][0], pair[0][1]), (pair[1][0], pair[1][1]), Rgba::hex(0x1b1b1b), 2.0 * s);
        }
    } else if let Some((x1, y1, x2, y2)) = state.selection {
        let r = Rect { x0: x1.min(x2), y0: y1.min(y2), x1: x1.max(x2), y1: y1.max(y2) };
        p.stroke_round(r, 0.0, theme.accent, 2.0 * s);
    }
    p.pop_clip();
    drew
}

/// Moves rendered tiles into the cache as bitmaps for this target.
pub(super) fn upload(state: &mut State, painter: &Painter) -> windows::core::Result<()> {
    for (key, frame, fresh) in std::mem::take(&mut state.arrived) {
        let bitmap = painter.upload(&frame)?;
        state.cache.insert(key, Tile { bitmap, fresh }, frame.pixels.len());
    }
    Ok(())
}

/// Background work for the view, most urgent first: tiles in view (after
/// their pages' placeholder thumbnails once the first page is up), then a
/// half-screen margin above and below, then the sidebar.
pub(super) fn wanted(state: &State, layout: &widgets::Layout) -> Vec<Item> {
    let Some(g) = geometry_in(state, layout.document) else {
        return predecode(state, layout.document);
    };
    let Some(v) = state.pdf.as_ref() else {
        return Vec::new();
    };
    if v.render_scale <= 0.0 {
        return Vec::new();
    }
    let render = v.render_scale;
    let ratio = g.scale / render;
    let item = |work: Work, scale: f32, region: [u32; 4]| Item {
        key: Key { doc: v.doc, work },
        path: v.path.clone(),
        edits: Arc::clone(&v.edits),
        scale,
        region,
    };
    let skip = |work: Work| {
        let key = Key { doc: v.doc, work };
        v.failed.contains(&key) || state.cache.peek(&key).is_some_and(|t| t.fresh)
    };
    let thumb = |page: u32| {
        let (w, h) = widgets::thumb_size(v.sizes[page as usize], state.scale);
        item(Work::Thumb { page }, 0.0, [0, 0, w as u32, h as u32])
    };
    let mid = (g.left + g.doc.width() / 2.0, g.top + g.doc.height() / 2.0);
    let tiles_in = |top: f32, bottom: f32| -> Vec<Item> {
        let mut found = Vec::new();
        for (page, r) in &g.layout.pages[view::visible(&g.layout, top, bottom)] {
            let clip = Rect {
                x0: (g.left - r.x0) / ratio,
                y0: (top - r.y0) / ratio,
                x1: (g.left + g.doc.width() - r.x0) / ratio,
                y1: (bottom - r.y0) / ratio,
            };
            for (col, row, region) in view::tiles(view::page_px(v.sizes[*page as usize], render), clip) {
                let work = Work::Tile { page: *page, scale: render.to_bits(), col, row };
                if !skip(work) {
                    let center = (r.x0 + (region[0] as f32 + region[2] as f32 / 2.0) * ratio, r.y0 + (region[1] as f32 + region[3] as f32 / 2.0) * ratio);
                    let distance = (center.0 - mid.0).hypot(center.1 - mid.1);
                    found.push((distance, item(work, render, region)));
                }
            }
        }
        found.sort_by(|a, b| a.0.total_cmp(&b.0));
        found.into_iter().map(|(_, i)| i).collect()
    };
    let shown = view::visible(&g.layout, g.top, g.top + g.doc.height());
    let pages: Vec<u32> = g.layout.pages[shown].iter().map(|(p, _)| *p).collect();
    let placeholders: Vec<Item> = pages
        .iter()
        .filter(|p| {
            let key = Key { doc: v.doc, work: Work::Thumb { page: **p } };
            state.cache.peek(&key).is_none() && !v.failed.contains(&key)
        })
        .map(|p| thumb(*p))
        .collect();
    let visible = tiles_in(g.top, g.top + g.doc.height());
    let mut out = Vec::new();
    if state.content_drawn {
        out.extend(placeholders);
        out.extend(visible);
    } else {
        // The first page goes up at full resolution first.
        out.extend(visible);
        out.extend(placeholders);
    }
    let margin = g.doc.height() / 2.0;
    let queued: HashSet<Key> = out.iter().map(|i| i.key).collect();
    out.extend(tiles_in(g.top - margin, g.top + g.doc.height() + margin).into_iter().filter(|i| !queued.contains(&i.key)));
    out.extend(pages.iter().filter(|p| !skip(Work::Thumb { page: **p }) && !queued.contains(&thumb(**p).key)).map(|p| thumb(*p)));
    if layout.sidebar.is_some() {
        match state.sidebar_tab {
            0 => {
                for w in &layout.widgets {
                    if let widgets::WidgetId::SidebarItem(index) = w.id {
                        let page = index as u32;
                        if !skip(Work::Thumb { page }) && !out.iter().any(|i| i.key.work == Work::Thumb { page }) {
                            out.push(thumb(page));
                        }
                    }
                }
            }
            1 if v.outline.is_none() => out.push(item(Work::Outline, 0.0, [0; 4])),
            2 => out.extend(
                v.scanned.iter().enumerate().filter(|(_, done)| !**done).take(32).map(|(page, _)| item(Work::Notes { page: page as u32 }, 0.0, [0; 4])),
            ),
            _ => {}
        }
    }
    out
}

/// After an image shows, its neighbors decode at the fit size, at a lower
/// priority than anything the view needs.
fn predecode(state: &State, doc: Rect) -> Vec<Item> {
    let Some((path, _)) = state.displayed.as_ref() else {
        return Vec::new();
    };
    if state.frame.is_none() || state.pending || state.render_failed || state.path.as_ref() != Some(path) {
        return Vec::new();
    }
    let (width, height) = ((doc.width() as u32).clamp(1, 4096), (doc.height() as u32).clamp(1, 4096));
    [1, -1]
        .into_iter()
        .map(|delta| Item {
            key: Key { doc: doc_id(path), work: Work::Predecode { delta, width, height } },
            path: path.clone(),
            edits: Arc::default(),
            scale: 0.0,
            region: [0; 4],
        })
        .collect()
}

/// Sends the background work list when it changed since the last frame.
pub(super) fn request(state: &mut State, layout: &widgets::Layout) {
    let items = wanted(state, layout);
    let keys: Vec<Key> = items.iter().map(|i| i.key).collect();
    if keys != state.sent {
        state.workers.request(items);
        state.sent = keys;
    }
}

/// Handles a pointer event that started on the document. Returns true when
/// the view needs a repaint.
pub(super) unsafe fn on_pointer(hwnd: HWND, state: &mut State, e: &PointerEvent) -> bool {
    if e.kind == PointerKind::Touch && touch(state, e) {
        return true;
    }
    let selecting = state.crop || state.markup.is_some() || state.zoom_select;
    match e.phase {
        Phase::Down => {
            if state.pending || state.render_failed || (state.frame.is_none() && state.pdf.is_none()) {
                return false;
            }
            if (state.crop || state.markup.is_some()) && state.pdf.is_some() {
                // Markup and crop go to the page under the pointer.
                let Some((page, rect)) = geometry(state).and_then(|g| page_at(&g, e.x, e.y)) else {
                    return false;
                };
                state.page = page;
                state.image_rect = rect;
                state.status = state.subtitle();
                if let Some(v) = state.pdf.as_mut() {
                    v.scroll = None;
                }
            }
            state.drag = Some((e.x, e.y));
            state.ink = vec![[e.x, e.y]];
            if selecting {
                state.selection = Some((e.x, e.y, e.x, e.y));
            }
            if e.kind == PointerKind::Mouse {
                SetCapture(hwnd);
            }
            false
        }
        Phase::Move => {
            let Some(start) = state.drag else {
                return false;
            };
            if selecting {
                state.selection = Some((start.0, start.1, e.x, e.y));
                if state.ink.len() < 10000 {
                    state.ink.push([e.x, e.y]);
                }
            } else {
                let (dx, dy) = (e.x - start.0, e.y - start.1);
                if state.pdf.is_some() {
                    scroll_by(state, -dx, -dy, false);
                } else {
                    state.pan.0 += dx;
                    state.pan.1 += dy;
                }
                state.drag = Some((e.x, e.y));
            }
            true
        }
        Phase::Cancel => {
            let _ = ReleaseCapture();
            state.drag = None;
            state.selection = None;
            state.ink.clear();
            true
        }
        Phase::Up => {
            let _ = ReleaseCapture();
            if state.drag.take().is_none() {
                return false;
            }
            finish_markup(hwnd, state);
            finish_crop(hwnd, state);
            finish_zoom(state);
            true
        }
    }
}

/// Two fingers pinch-zoom and pan around their midpoint; one finger acts
/// like the mouse.
fn touch(state: &mut State, e: &PointerEvent) -> bool {
    match e.phase {
        Phase::Down => {
            state.touches.retain(|(id, _)| *id != e.id);
            state.touches.push((e.id, (e.x, e.y)));
            if state.touches.len() == 2 && state.pinch.is_none() {
                state.drag = None;
                state.selection = None;
                state.ink.clear();
                let (a, b) = (state.touches[0], state.touches[1]);
                let (scale, origin) = match geometry(state) {
                    Some(g) => (g.scale, (g.left, g.top)),
                    None => match image_view(state) {
                        Some((doc, source)) => (view::image_scale(state.zoom, (doc.width(), doc.height()), source), state.pan),
                        None => return false,
                    },
                };
                state.pinch = Some(Pinch { ids: [a.0, b.0], start: [a.1, b.1], scale, origin });
                return true;
            }
            false
        }
        Phase::Move => {
            if let Some(point) = state.touches.iter_mut().find(|(id, _)| *id == e.id) {
                point.1 = (e.x, e.y);
            }
            let Some(pinch) = state.pinch else {
                return false;
            };
            let find = |id: u32| state.touches.iter().find(|(t, _)| *t == id).map(|(_, p)| *p);
            if let (Some(a), Some(b)) = (find(pinch.ids[0]), find(pinch.ids[1])) {
                let (ratio, (dx, dy)) = pinch_transform(pinch.start, [a, b]);
                pinch_to(state, &pinch, ratio, (dx, dy));
            }
            true
        }
        Phase::Up | Phase::Cancel => {
            state.touches.retain(|(id, _)| *id != e.id);
            if state.pinch.take().is_some() {
                // Render again at the new zoom once the fingers lift.
                state.due = Some(Instant::now() + SETTLE);
                return true;
            }
            false
        }
    }
}

fn pinch_to(state: &mut State, pinch: &Pinch, ratio: f32, (dx, dy): (f32, f32)) {
    let mid = ((pinch.start[0].0 + pinch.start[1].0) / 2.0, (pinch.start[0].1 + pinch.start[1].1) / 2.0);
    let actual = view::actual(state.scale);
    if let Some(v) = state.pdf.as_ref() {
        let gap = gap(state);
        let old = view::layout(&v.sizes, state.view_mode, state.page, pinch.scale, gap);
        let target = (pinch.scale * ratio / actual).clamp(view::MIN_RATIO, view::MAX_RATIO);
        state.zoom = Zoom::Ratio(target);
        let Some(g) = geometry(state) else {
            return;
        };
        let (x, y) = view::anchor(&old, pinch.origin, &g.layout, (mid.0 - g.doc.x0, mid.1 - g.doc.y0));
        if let Some(v) = state.pdf.as_mut() {
            v.scroll_x = x - dx;
            v.scroll = None;
        }
        set_top(state, &g, y - dy);
    } else if let Some((doc, _)) = image_view(state) {
        let scale = (pinch.scale * ratio).clamp(view::MIN_RATIO, view::MAX_RATIO);
        state.zoom = Zoom::Ratio(scale);
        let pan = view::image_pan(doc, pinch.origin, scale / pinch.scale, mid);
        state.pan = (pan.0 + dx, pan.1 + dy);
    }
}

/// Zoom to a dragged rectangle: it fills the view, centered.
fn finish_zoom(state: &mut State) {
    if !state.zoom_select {
        return;
    }
    let Some((x1, y1, x2, y2)) = state.selection.take() else {
        return;
    };
    state.zoom_select = false;
    let (w, h) = ((x1 - x2).abs(), (y1 - y2).abs());
    let doc = state.layout().document;
    if w < 8.0 || h < 8.0 {
        state.status = "Drag a larger area to zoom.".into();
        return;
    }
    let factor = (doc.width() / w).min(doc.height() / h);
    let at = ((x1 + x2) / 2.0, (y1 + y2) / 2.0);
    zoom_by(state, factor, Some(at));
    let (cx, cy) = center(doc);
    if state.pdf.is_some() {
        scroll_by(state, at.0 - cx, at.1 - cy, false);
    } else {
        state.pan = (state.pan.0 - (at.0 - cx), state.pan.1 - (at.1 - cy));
    }
    state.status = state.subtitle();
}

unsafe fn finish_markup(hwnd: HWND, state: &mut State) {
    if state.markup.is_none() {
        return;
    }
    let (Some(kind), Some((x1, y1, x2, y2)), Some(path)) = (state.markup, state.selection.take(), state.path.clone())
    else {
        return;
    };
    let rect = state.image_rect;
    let (width, height) = (rect.width(), rect.height());
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let normalized =
        |x: f32, y: f32| [((x - rect.x0) / width).clamp(0.0, 1.0), ((y - rect.y0) / height).clamp(0.0, 1.0)];
    let first = normalized(x1, y1);
    let mut last = normalized(x2, y2);
    if matches!(kind, AnnotationKind::Note | AnnotationKind::Text) && (x1 - x2).abs() < 3.0 {
        last = [(first[0] + 0.25).min(1.0), (first[1] + 0.08).min(1.0)];
    }
    let points = if let Some(signature) = &state.signature {
        signature
            .iter()
            .map(|p| {
                [
                    first[0].min(last[0]) + p[0] * (first[0] - last[0]).abs(),
                    first[1].min(last[1]) + p[1] * (first[1] - last[1]).abs(),
                ]
            })
            .collect()
    } else if kind == AnnotationKind::Ink {
        state.ink.iter().map(|p| normalized(p[0], p[1])).collect()
    } else {
        vec![first, last]
    };
    let pdf = state.is_pdf();
    let page = state.page;
    let text = state.markup_text.clone();
    let edits = state.sessions.entry(path).or_default();
    if pdf {
        edits.pdf.push(PdfEdit::Annotate { page, kind, points, text });
    } else {
        edits.image.push(ImageEdit::Annotate { kind, points, text });
    }
    edits.dirty = true;
    state.ink.clear();
    schedule(hwnd, state, 0);
}

unsafe fn finish_crop(hwnd: HWND, state: &mut State) {
    if !state.crop {
        return;
    }
    let (Some((x1, y1, x2, y2)), Some(path)) = (state.selection.take(), state.path.clone()) else {
        return;
    };
    let rect = state.image_rect;
    let (width, height) = (rect.width(), rect.height());
    if width <= 0.0 || height <= 0.0 || (x1 - x2).abs() <= 3.0 || (y1 - y2).abs() <= 3.0 {
        return;
    }
    let left = ((x1.min(x2) - rect.x0) / width).clamp(0.0, 1.0);
    let right = ((x1.max(x2) - rect.x0) / width).clamp(0.0, 1.0);
    let top = ((y1.min(y2) - rect.y0) / height).clamp(0.0, 1.0);
    let bottom = ((y1.max(y2) - rect.y0) / height).clamp(0.0, 1.0);
    if right <= left || bottom <= top {
        return;
    }
    let pdf = state.is_pdf();
    let page = state.page;
    let edits = state.sessions.entry(path).or_default();
    if pdf {
        edits.pdf.push(PdfEdit::Crop { page, left, top, right, bottom });
    } else {
        edits.image.push(ImageEdit::Crop { left, top, right, bottom });
        state.pan = (0.0, 0.0);
        state.zoom = Zoom::Fit;
    }
    edits.dirty = true;
    state.crop = false;
    schedule(hwnd, state, 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::Frame, ui::theme, ui::worker::Workers};

    const LETTER: [f32; 2] = [612.0, 792.0];

    /// A window 1100 by 800 pixels showing a 20-page PDF.
    fn pdf_state(zoom: Zoom) -> State {
        let workers = Workers::start(HWND::default()).unwrap();
        let path = PathBuf::from("a.pdf");
        let mut s = State::new(workers, &[path.clone()], (1100.0, 800.0), 1.0, 1.0, theme::palette(theme::Mode::Light));
        s.animations = false;
        s.zoom = zoom;
        s.pdf = Some(PdfView::new(path, vec![LETTER; 20], Arc::default(), 0));
        let doc = s.layout().document;
        prepare(&mut s, doc);
        s
    }

    /// The page under a client point, and the point as a fraction of it.
    fn under(s: &State, at: (f32, f32)) -> (u32, f32, f32) {
        let g = geometry(s).unwrap();
        let (page, r) = g.layout.pages.iter().map(|(p, r)| (*p, screen(&g, *r))).find(|(_, r)| r.contains(at.0, at.1)).unwrap();
        (page, (at.0 - r.x0) / r.width(), (at.1 - r.y0) / r.height())
    }

    fn top(s: &State) -> f32 {
        geometry(s).unwrap().top
    }

    #[test]
    fn scroll_keys_move_by_lines_and_screens_and_stop_at_the_ends() {
        let mut s = pdf_state(Zoom::FitWidth);
        let doc = s.layout().document;
        let start = top(&s);
        assert!(key(&mut s, VK_DOWN.0, false));
        assert_eq!(top(&s), start + LINE);
        assert!(key(&mut s, VK_NEXT.0, false));
        assert_eq!(top(&s), start + LINE + doc.height() - LINE);
        assert!(key(&mut s, VK_SPACE.0, true), "Shift+Space goes back a screen");
        assert_eq!(top(&s), start + LINE);
        assert!(key(&mut s, VK_END.0, false));
        let g = geometry(&s).unwrap();
        assert_eq!(g.top, g.layout.height - doc.height());
        assert_eq!(s.page, 19);
        assert_eq!(s.status, "Page 20 of 20");
        assert!(key(&mut s, VK_HOME.0, false));
        assert_eq!((top(&s), s.page), (0.0, 0));
        assert!(!key(&mut s, VK_LEFT.0, false), "Left stays with the Previous command");
        assert!(step_page(&mut s, 1) && s.page == 1);
        assert!(!step_page(&mut s, -2), "no page before the first");
    }

    #[test]
    fn single_page_view_turns_the_page_at_its_edges() {
        let mut s = pdf_state(Zoom::FitWidth);
        s.view_mode = ViewMode::Single;
        go_to_page(&mut s, 3, false);
        let mut presses = 0;
        while s.page == 3 && presses < 10 {
            key(&mut s, VK_NEXT.0, false);
            presses += 1;
        }
        assert_eq!((s.page, presses), (4, 3), "two screens of page 4, then the next page");
        key(&mut s, VK_PRIOR.0, false);
        key(&mut s, VK_PRIOR.0, false);
        assert_eq!(s.page, 3);
        let g = geometry(&s).unwrap();
        assert_eq!(g.top, g.layout.height - g.doc.height(), "the previous page shows its bottom");
        key(&mut s, VK_END.0, false);
        assert_eq!(s.page, 19);
    }

    #[test]
    fn two_page_view_steps_a_row_at_a_time() {
        let mut s = pdf_state(Zoom::Fit);
        s.view_mode = ViewMode::TwoPages;
        assert!(step_page(&mut s, 1));
        assert_eq!(s.page, 2);
        s.page = 5;
        assert!(step_page(&mut s, -1));
        assert_eq!(s.page, 2, "back from the row of pages 5 and 6");
    }

    #[test]
    fn zoom_keeps_the_page_point_under_the_pointer() {
        let mut s = pdf_state(Zoom::FitWidth);
        scroll_by(&mut s, 0.0, 2000.0, false);
        let at = (700.0, 500.0);
        let before = under(&s, at);
        set_zoom(&mut s, Zoom::Ratio(2.5), Some(at));
        let after = under(&s, at);
        assert_eq!(before.0, after.0);
        assert!((before.1 - after.1).abs() < 0.002 && (before.2 - after.2).abs() < 0.002, "{before:?} {after:?}");
        let r = ratio(&s);
        wheel(&mut s, 120.0, false, true, at);
        assert!((ratio(&s) / r - 1.25).abs() < 1e-3, "Ctrl+wheel zooms by a quarter per notch");
        let wheeled = under(&s, at);
        assert!((before.1 - wheeled.1).abs() < 0.002 && (before.2 - wheeled.2).abs() < 0.002);
        set_zoom(&mut s, Zoom::Ratio(0.8), None);
        zoom_step(&mut s, true);
        assert!((ratio(&s) - 1.0).abs() < 1e-4, "zoom in lands on actual size");
        set_zoom(&mut s, Zoom::Fit, None);
        let g = geometry(&s).unwrap();
        let page = g.layout.pages.iter().find(|(p, _)| *p == s.page).unwrap().1;
        assert!(page.height() <= g.doc.height(), "fit to window shows the whole page");
    }

    #[test]
    fn pinch_zooms_around_the_fingers_and_settles_after() {
        let mut s = pdf_state(Zoom::FitWidth);
        let g = geometry(&s).unwrap();
        let fingers = [(400.0, 400.0), (600.0, 400.0)];
        let before = under(&s, (500.0, 400.0));
        let pinch = Pinch { ids: [1, 2], start: fingers, scale: g.scale, origin: (g.left, g.top) };
        pinch_to(&mut s, &pinch, 2.0, (0.0, 0.0));
        assert!((geometry(&s).unwrap().scale / g.scale - 2.0).abs() < 1e-3);
        let after = under(&s, (500.0, 400.0));
        assert!(before.0 == after.0 && (before.1 - after.1).abs() < 0.002 && (before.2 - after.2).abs() < 0.002);
        let doc = s.layout().document;
        prepare(&mut s, doc);
        assert!(s.due.is_some(), "tiles at the new scale wait until the zoom is still");
        assert!(settle(&mut s));
        let v = s.pdf.as_ref().unwrap();
        assert_eq!((v.render_scale, v.fallback_scale), (geometry(&s).unwrap().scale, g.scale));
    }

    #[test]
    fn zoom_to_selection_fills_the_view_with_the_dragged_area() {
        let mut s = pdf_state(Zoom::FitWidth);
        let doc = s.layout().document;
        let (x, y) = (300.0, 300.0);
        let target = under(&s, (x + 100.0, y + 50.0));
        let start = ratio(&s);
        s.zoom_select = true;
        s.selection = Some((x, y, x + 200.0, y + 100.0));
        finish_zoom(&mut s);
        assert!(!s.zoom_select);
        let center = ((doc.x0 + doc.x1) / 2.0, (doc.y0 + doc.y1) / 2.0);
        let shown = under(&s, center);
        assert_eq!(shown.0, target.0);
        assert!((shown.1 - target.1).abs() < 0.003 && (shown.2 - target.2).abs() < 0.003, "{shown:?} {target:?}");
        let factor = (doc.width() / 200.0).min(doc.height() / 100.0);
        assert!((ratio(&s) - start * factor).abs() < 1e-3, "the 200-pixel-wide area now fills the view");
    }

    #[test]
    fn images_zoom_around_the_pointer_and_decode_again_when_still() {
        let workers = Workers::start(HWND::default()).unwrap();
        let path = PathBuf::from("photo.jpg");
        let mut s = State::new(workers, &[path.clone()], (1100.0, 800.0), 1.0, 1.0, theme::palette(theme::Mode::Light));
        s.frame = Some(Frame { width: 1, height: 1, pixels: vec![255; 4], page_count: 1, source_width: 4000, source_height: 3000 });
        s.displayed = Some((path, 0));
        let doc = s.layout().document;
        assert_eq!(image_box(&s, doc), (doc.width() as u32, doc.height() as u32), "fit decodes at the view size");
        let at = (300.0, 300.0);
        let before = view::image_rect(doc, (4000, 3000), ratio(&s), s.pan);
        set_zoom(&mut s, Zoom::Ratio(0.5), Some(at));
        let after = view::image_rect(doc, (4000, 3000), 0.5, s.pan);
        let fraction = |r: Rect| ((at.0 - r.x0) / r.width(), (at.1 - r.y0) / r.height());
        assert!((fraction(before).0 - fraction(after).0).abs() < 1e-4 && (fraction(before).1 - fraction(after).1).abs() < 1e-4);
        assert!(s.due.is_some());
        assert_eq!(image_box(&s, doc), (2000, 1500), "decoded at the shown size once still");
        set_zoom(&mut s, Zoom::Fit, None);
        assert_eq!(s.pan, (0.0, 0.0));
    }

    #[test]
    fn pinch_ratio_and_midpoint_pan() {
        let start = [(100.0, 100.0), (200.0, 100.0)];
        assert_eq!(pinch_transform(start, start), (1.0, (0.0, 0.0)));
        let (ratio, pan) = pinch_transform(start, [(50.0, 100.0), (250.0, 100.0)]);
        assert_eq!((ratio, pan), (2.0, (0.0, 0.0)));
        let (ratio, pan) = pinch_transform(start, [(110.0, 120.0), (210.0, 120.0)]);
        assert_eq!((ratio, pan), (1.0, (10.0, 20.0)));
        assert_eq!(pinch_transform([(5.0, 5.0), (5.0, 5.0)], start).0, 1.0, "coincident fingers do not divide by zero");
    }
}
