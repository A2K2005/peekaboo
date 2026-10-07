//! Page layout, tiles, scrolling, and zoom math for the PDF view, and the
//! zoom model shared with images. Pure functions in device pixels, so all of
//! it runs in unit tests.
use super::widgets::Rect;
use std::ops::Range;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ViewMode {
    #[default]
    Continuous,
    Single,
    TwoPages,
}

/// Fit modes follow the window size. `Ratio` is a fraction of actual size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) enum Zoom {
    #[default]
    Fit,
    FitWidth,
    Ratio(f32),
}

/// Tile side in device pixels.
pub(super) const TILE: u32 = 512;
pub(super) const MIN_RATIO: f32 = 0.1;
pub(super) const MAX_RATIO: f32 = 8.0;
/// Zoom steps as a fraction of actual size. 1.0 is a stop, so Ctrl+= lands
/// on actual size (docs/research/ux-teardown.md, D1).
const STEPS: [f32; 15] = [0.1, 0.15, 0.25, 0.33, 0.5, 0.67, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0];

/// The next zoom step above or below `ratio`.
pub(super) fn step(ratio: f32, up: bool) -> f32 {
    if up {
        STEPS.iter().copied().find(|s| *s > ratio * 1.01).unwrap_or(MAX_RATIO)
    } else {
        STEPS.iter().rev().copied().find(|s| *s < ratio * 0.99).unwrap_or(MIN_RATIO)
    }
}

/// Device pixels per PDF point at actual size: 96 px per inch, 72 points
/// per inch, times the display scale.
pub(super) fn actual(scale: f32) -> f32 {
    scale * 96.0 / 72.0
}

/// A page's pixel size at `scale`, rounded as `PdfEngine::render_region` does.
pub(super) fn page_px(size: [f32; 2], scale: f32) -> (u32, u32) {
    ((size[0] * scale).round().max(1.0) as u32, (size[1] * scale).round().max(1.0) as u32)
}

fn per_row(mode: ViewMode) -> usize {
    if mode == ViewMode::TwoPages {
        2
    } else {
        1
    }
}

/// Device pixels per point for a PDF. Fit modes use the widest and tallest
/// row, so the scale stays put while scrolling through mixed page sizes.
pub(super) fn pdf_scale(sizes: &[[f32; 2]], mode: ViewMode, zoom: Zoom, view: (f32, f32), gap: f32, actual: f32) -> f32 {
    let ratio = match zoom {
        Zoom::Ratio(r) => r,
        fit => {
            let mut scale = f32::INFINITY;
            for row in sizes.chunks(per_row(mode)) {
                let width: f32 = row.iter().map(|s| s[0]).sum();
                let height = row.iter().map(|s| s[1]).fold(0.0, f32::max);
                let room = view.0 - 2.0 * gap - gap * (row.len() - 1) as f32;
                scale = scale.min(room / width.max(1.0));
                if fit == Zoom::Fit {
                    scale = scale.min((view.1 - 2.0 * gap) / height.max(1.0));
                }
            }
            if !scale.is_finite() {
                return actual;
            }
            scale / actual
        }
    };
    ratio.clamp(MIN_RATIO, MAX_RATIO) * actual
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Row {
    /// Indices into `Layout::pages`.
    pub(super) pages: Range<usize>,
    pub(super) y0: f32,
    pub(super) y1: f32,
}

/// Laid-out pages in content pixels, top to bottom. Single-page mode lays
/// out only `current`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Layout {
    pub(super) pages: Vec<(u32, Rect)>,
    pub(super) rows: Vec<Row>,
    pub(super) width: f32,
    pub(super) height: f32,
}

pub(super) fn layout(sizes: &[[f32; 2]], mode: ViewMode, current: u32, scale: f32, gap: f32) -> Layout {
    if sizes.is_empty() {
        return Layout::default();
    }
    let indices: Vec<u32> = match mode {
        ViewMode::Single => vec![current.min(sizes.len() as u32 - 1)],
        _ => (0..sizes.len() as u32).collect(),
    };
    let px = |page: u32| page_px(sizes[page as usize], scale);
    let row_width = |row: &[u32]| row.iter().map(|p| px(*p).0 as f32).sum::<f32>() + gap * (row.len() - 1) as f32;
    let rows = indices.chunks(per_row(mode));
    let width = rows.clone().map(row_width).fold(0.0, f32::max) + 2.0 * gap;
    let mut out = Layout { width, ..Default::default() };
    let mut y = gap;
    for row in rows {
        let height = row.iter().map(|p| px(*p).1 as f32).fold(0.0, f32::max);
        let mut x = ((width - row_width(row)) / 2.0).round();
        let start = out.pages.len();
        for page in row {
            let (w, h) = px(*page);
            out.pages.push((*page, Rect::new(x, y, w as f32, h as f32)));
            x += w as f32 + gap;
        }
        out.rows.push(Row { pages: start..out.pages.len(), y0: y, y1: y + height });
        y += height + gap;
    }
    out.height = y;
    out
}

/// Pages (indices into `layout.pages`) whose rows overlap `top..bottom`.
pub(super) fn visible(layout: &Layout, top: f32, bottom: f32) -> Range<usize> {
    let first = layout.rows.partition_point(|r| r.y1 <= top);
    let last = layout.rows.partition_point(|r| r.y0 < bottom);
    if first >= last {
        return 0..0;
    }
    layout.rows[first].pages.start..layout.rows[last - 1].pages.end
}

/// The page under the view's vertical center: the first page of the first
/// row that ends below it.
pub(super) fn current(layout: &Layout, center: f32) -> Option<u32> {
    let row = layout.rows.partition_point(|r| r.y1 <= center).min(layout.rows.len().checked_sub(1)?);
    layout.pages.get(layout.rows[row].pages.start).map(|(page, _)| *page)
}

/// Where the view starts in content pixels. Content smaller than the view
/// is centered, so the offset goes negative.
pub(super) fn origin(content: f32, view: f32, scroll: f32) -> f32 {
    if content <= view {
        ((content - view) / 2.0).round()
    } else {
        scroll.clamp(0.0, content - view)
    }
}

/// The view top as a page and an offset in points from that page's top.
/// It survives any layout change: zoom, resize, and view mode.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Position {
    pub(super) page: u32,
    pub(super) offset: f32,
}

pub(super) fn to_y(layout: &Layout, position: Position, scale: f32) -> f32 {
    match layout.pages.iter().position(|(page, _)| *page == position.page) {
        Some(index) => layout.pages[index].1.y0 + position.offset * scale,
        None => 0.0,
    }
}

pub(super) fn from_y(layout: &Layout, y: f32, scale: f32) -> Position {
    let index = layout.pages.partition_point(|(_, r)| r.y0 <= y).saturating_sub(1);
    match layout.pages.get(index) {
        Some((page, rect)) => Position { page: *page, offset: (y - rect.y0) / scale.max(1e-6) },
        None => Position::default(),
    }
}

/// The page nearest to a content point, and the point as a fraction of that
/// page's rectangle (outside 0..1 in the gaps).
fn locate(layout: &Layout, point: (f32, f32)) -> Option<(u32, (f32, f32))> {
    let row = layout.rows.partition_point(|r| r.y1 <= point.1).min(layout.rows.len().checked_sub(1)?);
    let distance = |r: &Rect| if point.0 < r.x0 { r.x0 - point.0 } else { (point.0 - r.x1).max(0.0) };
    let (page, rect) = layout.pages[layout.rows[row].pages.clone()]
        .iter()
        .min_by(|a, b| distance(&a.1).total_cmp(&distance(&b.1)))?;
    Some((*page, ((point.0 - rect.x0) / rect.width().max(1.0), (point.1 - rect.y0) / rect.height().max(1.0))))
}

/// The view origin for `new` that keeps the content under `anchor` (view
/// pixels) in place, given the origin it had in `old`.
pub(super) fn anchor(old: &Layout, origin: (f32, f32), new: &Layout, anchor: (f32, f32)) -> (f32, f32) {
    let point = (origin.0 + anchor.0, origin.1 + anchor.1);
    let Some((page, (fx, fy))) = locate(old, point) else {
        return origin;
    };
    match new.pages.iter().find(|(p, _)| *p == page) {
        Some((_, r)) => (r.x0 + fx * r.width() - anchor.0, r.y0 + fy * r.height() - anchor.1),
        None => origin,
    }
}

/// Tiles of a page `px` pixels big that overlap `clip` (page pixels), as
/// (column, row, region). A region is `[x, y, width, height]`; edge tiles are
/// cut to the page.
pub(super) fn tiles(px: (u32, u32), clip: Rect) -> Vec<(u32, u32, [u32; 4])> {
    let (x0, y0) = (clip.x0.max(0.0), clip.y0.max(0.0));
    let (x1, y1) = (clip.x1.min(px.0 as f32), clip.y1.min(px.1 as f32));
    if x1 <= x0 || y1 <= y0 {
        return Vec::new();
    }
    let t = TILE as f32;
    let mut out = Vec::new();
    for row in (y0 / t) as u32..(y1 / t).ceil() as u32 {
        for col in (x0 / t) as u32..(x1 / t).ceil() as u32 {
            let (x, y) = (col * TILE, row * TILE);
            out.push((col, row, [x, y, TILE.min(px.0 - x), TILE.min(px.1 - y)]));
        }
    }
    out
}

/// Screen pixels per source pixel for an image. Fit never enlarges.
pub(super) fn image_scale(zoom: Zoom, view: (f32, f32), source: (u32, u32)) -> f32 {
    let (w, h) = (source.0.max(1) as f32, source.1.max(1) as f32);
    match zoom {
        Zoom::Fit => (view.0 / w).min(view.1 / h).min(1.0),
        Zoom::FitWidth => (view.0 / w).min(1.0),
        Zoom::Ratio(r) => r.clamp(MIN_RATIO, MAX_RATIO),
    }
}

/// The image's screen rectangle: centered in `doc`, then moved by `pan`.
pub(super) fn image_rect(doc: Rect, source: (u32, u32), scale: f32, pan: (f32, f32)) -> Rect {
    let (w, h) = (source.0 as f32 * scale, source.1 as f32 * scale);
    let left = doc.x0 + (doc.width() - w) / 2.0 + pan.0;
    let top = doc.y0 + (doc.height() - h) / 2.0 + pan.1;
    Rect { x0: left, y0: top, x1: left + w, y1: top + h }
}

/// The image pan after scaling by `factor` around `anchor` (client pixels),
/// so the image point under the anchor stays put.
pub(super) fn image_pan(doc: Rect, pan: (f32, f32), factor: f32, anchor: (f32, f32)) -> (f32, f32) {
    let center = ((doc.x0 + doc.x1) / 2.0, (doc.y0 + doc.y1) / 2.0);
    let image = (center.0 + pan.0, center.1 + pan.1);
    (
        anchor.0 - factor * (anchor.0 - image.0) - center.0,
        anchor.1 - factor * (anchor.1 - image.1) - center.1,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const LETTER: [f32; 2] = [612.0, 792.0];

    #[test]
    fn zoom_steps_stop_at_actual_size_and_stay_in_range() {
        assert_eq!(step(0.8, true), 1.0);
        assert_eq!(step(1.0, true), 1.25);
        assert_eq!(step(1.0, false), 0.75);
        assert_eq!(step(1.2, false), 1.0);
        assert_eq!(step(8.0, true), MAX_RATIO);
        assert_eq!(step(0.1, false), MIN_RATIO);
        let mut r = 0.1;
        for _ in 0..20 {
            let next = step(r, true);
            assert!(next >= r);
            r = next;
        }
        assert_eq!(r, MAX_RATIO);
    }

    #[test]
    fn continuous_layout_stacks_centered_pages_with_gaps() {
        let sizes = [LETTER, [792.0, 612.0], LETTER];
        let l = layout(&sizes, ViewMode::Continuous, 0, 1.0, 10.0);
        assert_eq!(l.pages.len(), 3);
        assert_eq!(l.width, 792.0 + 20.0);
        assert_eq!(l.pages[0].1, Rect::new(100.0, 10.0, 612.0, 792.0));
        assert_eq!(l.pages[1].1, Rect::new(10.0, 812.0, 792.0, 612.0));
        assert_eq!(l.pages[2].1.y0, 812.0 + 612.0 + 10.0);
        assert_eq!(l.height, l.pages[2].1.y1 + 10.0);
    }

    #[test]
    fn two_page_and_single_layouts() {
        let sizes = [LETTER; 5];
        let two = layout(&sizes, ViewMode::TwoPages, 0, 0.5, 10.0);
        assert_eq!(two.rows.len(), 3);
        assert_eq!(two.rows[0].pages, 0..2);
        let (left, right) = (two.pages[0].1, two.pages[1].1);
        assert_eq!((left.y0, right.y0), (10.0, 10.0));
        assert_eq!(right.x0, left.x1 + 10.0);
        assert_eq!(two.rows[2].pages, 4..5, "the odd last page sits alone");
        let single = layout(&sizes, ViewMode::Single, 3, 1.0, 10.0);
        assert_eq!(single.pages, vec![(3, Rect::new(10.0, 10.0, 612.0, 792.0))]);
        assert_eq!(layout(&sizes, ViewMode::Single, 99, 1.0, 10.0).pages[0].0, 4, "current is clamped");
        assert_eq!(layout(&[], ViewMode::Continuous, 0, 1.0, 10.0), Layout::default());
    }

    #[test]
    fn fit_scales_use_the_widest_row_and_clamp() {
        let sizes = [LETTER, LETTER];
        let a = actual(1.0);
        let width = pdf_scale(&sizes, ViewMode::Continuous, Zoom::FitWidth, (632.0, 400.0), 10.0, a);
        assert!((width - 1.0).abs() < 1e-4);
        let page = pdf_scale(&sizes, ViewMode::Continuous, Zoom::Fit, (632.0, 416.0), 10.0, a);
        assert!((page - 0.5).abs() < 1e-4, "{page}");
        let two = pdf_scale(&sizes, ViewMode::TwoPages, Zoom::FitWidth, (1254.0, 400.0), 10.0, a);
        assert!((two - 1.0).abs() < 1e-4, "two pages plus one inner gap");
        assert_eq!(pdf_scale(&sizes, ViewMode::Continuous, Zoom::Ratio(1.0), (1.0, 1.0), 10.0, a), a);
        assert_eq!(pdf_scale(&sizes, ViewMode::Continuous, Zoom::Ratio(100.0), (1.0, 1.0), 10.0, a), MAX_RATIO * a);
        assert_eq!(pdf_scale(&sizes, ViewMode::Continuous, Zoom::Fit, (1.0, 1.0), 10.0, a), MIN_RATIO * a);
        assert_eq!(actual(1.5), 2.0);
    }

    #[test]
    fn visible_range_finds_only_overlapping_rows_in_500_pages() {
        let sizes = vec![LETTER; 500];
        let l = layout(&sizes, ViewMode::Continuous, 0, 1.0, 10.0);
        let pitch = 792.0 + 10.0;
        assert_eq!(visible(&l, 0.0, 700.0), 0..1);
        assert_eq!(visible(&l, 700.0, 900.0), 0..2);
        assert_eq!(visible(&l, pitch * 250.0 + 20.0, pitch * 250.0 + 700.0), 250..251);
        assert_eq!(visible(&l, l.height + 10.0, l.height + 100.0), 0..0);
        let two = layout(&sizes, ViewMode::TwoPages, 0, 1.0, 10.0);
        assert_eq!(visible(&two, 0.0, 700.0), 0..2);
        assert_eq!(current(&l, pitch * 3.5), Some(3));
        assert_eq!(current(&two, pitch * 3.5), Some(6));
        assert_eq!(current(&l, l.height * 2.0), Some(499));
    }

    #[test]
    fn position_survives_zoom_and_resize() {
        let sizes = vec![LETTER; 20];
        let a = layout(&sizes, ViewMode::Continuous, 0, 1.0, 10.0);
        let p = from_y(&a, a.pages[5].1.y0 + 100.0, 1.0);
        assert_eq!(p, Position { page: 5, offset: 100.0 });
        let b = layout(&sizes, ViewMode::Continuous, 0, 2.0, 10.0);
        assert_eq!(to_y(&b, p, 2.0), b.pages[5].1.y0 + 200.0);
        let two = layout(&sizes, ViewMode::TwoPages, 0, 1.0, 10.0);
        let q = from_y(&two, two.pages[4].1.y0 + 50.0, 1.0);
        assert_eq!(to_y(&two, q, 1.0), two.pages[4].1.y0 + 50.0, "same row, same y");
        assert_eq!(from_y(&a, -5.0, 1.0).page, 0);
    }

    #[test]
    fn zoom_keeps_the_point_under_the_cursor() {
        let sizes = vec![LETTER; 10];
        let old = layout(&sizes, ViewMode::Continuous, 0, 1.0, 10.0);
        let new = layout(&sizes, ViewMode::Continuous, 0, 2.5, 10.0);
        let origin = (-200.0, 3000.0);
        let cursor = (500.0, 300.0);
        let (fx, fy) = locate(&old, (origin.0 + cursor.0, origin.1 + cursor.1)).unwrap().1;
        let shifted = anchor(&old, origin, &new, cursor);
        let (page, (gx, gy)) = locate(&new, (shifted.0 + cursor.0, shifted.1 + cursor.1)).unwrap();
        assert_eq!(page, 4);
        assert!((fx - gx).abs() < 1e-4 && (fy - gy).abs() < 1e-4);
        // Two-page rows: the left page keeps the point, not the right one.
        let two_old = layout(&sizes, ViewMode::TwoPages, 0, 1.0, 10.0);
        let two_new = layout(&sizes, ViewMode::TwoPages, 0, 0.5, 10.0);
        let origin = (-100.0, two_old.pages[2].1.y0);
        let before = locate(&two_old, (origin.0 + 150.0, origin.1)).unwrap();
        let moved = anchor(&two_old, origin, &two_new, (150.0, 0.0));
        let after = locate(&two_new, (moved.0 + 150.0, moved.1)).unwrap();
        assert_eq!((before.0, after.0), (2, 2));
        assert!((before.1 .0 - after.1 .0).abs() < 1e-4 && (before.1 .1 - after.1 .1).abs() < 1e-4);
    }

    #[test]
    fn tile_grid_covers_the_clip_and_cuts_edge_tiles() {
        let page = (1100, 1300);
        let all = tiles(page, Rect::new(0.0, 0.0, 5000.0, 5000.0));
        assert_eq!(all.len(), 9);
        assert_eq!(all[0], (0, 0, [0, 0, 512, 512]));
        assert_eq!(all[8], (2, 2, [1024, 1024, 76, 276]));
        let area: u32 = all.iter().map(|t| t.2[2] * t.2[3]).sum();
        assert_eq!(area, 1100 * 1300);
        assert_eq!(tiles(page, Rect::new(600.0, 520.0, 10.0, 10.0)), vec![(1, 1, [512, 512, 512, 512])]);
        assert_eq!(tiles(page, Rect::new(500.0, 0.0, 30.0, 10.0)).len(), 2, "a clip across a seam");
        assert!(tiles(page, Rect::new(2000.0, 0.0, 10.0, 10.0)).is_empty());
        assert!(tiles(page, Rect::new(-50.0, -50.0, 40.0, 40.0)).is_empty());
    }

    #[test]
    fn origin_centers_small_content_and_clamps_scrolling() {
        assert_eq!(origin(500.0, 700.0, 123.0), -100.0);
        assert_eq!(origin(1000.0, 700.0, -50.0), 0.0);
        assert_eq!(origin(1000.0, 700.0, 900.0), 300.0);
    }

    #[test]
    fn images_fit_without_enlarging_and_zoom_around_the_cursor() {
        assert_eq!(image_scale(Zoom::Fit, (1000.0, 500.0), (4000, 1000)), 0.25);
        assert_eq!(image_scale(Zoom::Fit, (1000.0, 500.0), (40, 10)), 1.0);
        assert_eq!(image_scale(Zoom::FitWidth, (1000.0, 500.0), (2000, 9000)), 0.5);
        assert_eq!(image_scale(Zoom::Ratio(2.0), (1.0, 1.0), (1, 1)), 2.0);
        let doc = Rect::new(0.0, 100.0, 1000.0, 600.0);
        let r = image_rect(doc, (400, 200), 1.0, (0.0, 0.0));
        assert_eq!(r, Rect::new(300.0, 300.0, 400.0, 200.0));
        let cursor = (350.0, 320.0);
        let pan = image_pan(doc, (0.0, 0.0), 2.0, cursor);
        let zoomed = image_rect(doc, (400, 200), 2.0, pan);
        let before = ((cursor.0 - r.x0) / r.width(), (cursor.1 - r.y0) / r.height());
        let after = ((cursor.0 - zoomed.x0) / zoomed.width(), (cursor.1 - zoomed.y0) / zoomed.height());
        assert!((before.0 - after.0).abs() < 1e-5 && (before.1 - after.1).abs() < 1e-5);
    }
}
