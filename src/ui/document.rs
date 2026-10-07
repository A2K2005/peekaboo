//! The document view: drawing the page or image, and pointer input on it.
//! Mouse, pen, and touch arrive as one `PointerEvent` type from WM_POINTER,
//! so wave 2 can route pen input (with pressure) straight to ink.
use super::{
    app::{schedule, State},
    render::Painter,
    theme::Rgba,
    widgets::Rect,
};
use crate::model::{AnnotationKind, ImageEdit, PdfEdit};
use std::time::{Duration, Instant};
use windows::Win32::{
    Foundation::HWND,
    Graphics::Direct2D::ID2D1Bitmap,
    UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture},
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
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Pinch {
    ids: [u32; 2],
    start: [(f32, f32); 2],
    zoom: f32,
    pan: (f32, f32),
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

pub(super) const MIN_ZOOM: f32 = 0.25;
pub(super) const MAX_ZOOM: f32 = 8.0;

/// Where the frame goes inside the document rectangle. Frames render at the
/// requested zoom; between renders, `zoom / frame_zoom` scales the old frame.
pub(super) fn frame_rect(doc: Rect, frame: (u32, u32), zoom: f32, frame_zoom: f32, pan: (f32, f32)) -> Rect {
    let (w, h) = (frame.0 as f32, frame.1 as f32);
    let base = if frame_zoom > 1.0 { 1.0 } else { (doc.width() / w).min(doc.height() / h).min(1.0) };
    let scale = base * if frame_zoom > 0.0 { zoom / frame_zoom } else { 1.0 };
    let (w, h) = (w * scale, h * scale);
    let left = doc.x0 + (doc.width() - w) / 2.0 + pan.0;
    let top = doc.y0 + (doc.height() - h) / 2.0 + pan.1;
    Rect { x0: left, y0: top, x1: left + w, y1: top + h }
}

/// Draws the canvas, the frame, and in-progress markup. Returns true when
/// the frame was drawn (the launch benchmark marker needs real content).
pub(super) fn paint(p: &Painter, bitmap: Option<&ID2D1Bitmap>, state: &mut State, doc: Rect) -> bool {
    let theme = state.theme;
    p.fill(doc, theme.canvas);
    let (Some(frame), Some(bitmap)) = (state.frame.as_ref(), bitmap) else {
        return false;
    };
    let rect = frame_rect(doc, (frame.width, frame.height), state.zoom, state.frame_zoom, state.pan);
    state.image_rect = rect;
    p.push_clip(doc);
    p.draw_bitmap(bitmap, rect);
    let s = state.scale;
    let outline = Rect { x0: rect.x0 - s, y0: rect.y0 - s, x1: rect.x1 + s, y1: rect.y1 + s };
    p.stroke_round(outline, 0.0, theme.border, s);
    if state.markup == Some(AnnotationKind::Ink) && state.signature.is_none() {
        for pair in state.ink.windows(2) {
            p.line((pair[0][0], pair[0][1]), (pair[1][0], pair[1][1]), Rgba::hex(0x1b1b1b), 2.0 * s);
        }
    } else if let Some((x1, y1, x2, y2)) = state.selection {
        let r = Rect { x0: x1.min(x2), y0: y1.min(y2), x1: x1.max(x2), y1: y1.max(y2) };
        p.stroke_round(r, 0.0, theme.accent, 2.0 * s);
    }
    p.pop_clip();
    true
}

/// Handles a pointer event that started on the document. Returns true when
/// the view needs a repaint.
pub(super) unsafe fn on_pointer(hwnd: HWND, state: &mut State, e: &PointerEvent) -> bool {
    if e.kind == PointerKind::Touch && touch(state, e) {
        return true;
    }
    match e.phase {
        Phase::Down => {
            if state.pending || state.render_failed || state.frame.is_none() {
                return false;
            }
            state.drag = Some((e.x, e.y));
            state.ink = vec![[e.x, e.y]];
            if state.crop || state.markup.is_some() {
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
            if state.crop || state.markup.is_some() {
                state.selection = Some((start.0, start.1, e.x, e.y));
                if state.ink.len() < 10000 {
                    state.ink.push([e.x, e.y]);
                }
            } else {
                state.pan.0 += e.x - start.0;
                state.pan.1 += e.y - start.1;
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
            true
        }
    }
}

/// Two fingers pinch-zoom and pan; one finger acts like the mouse.
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
                state.pinch = Some(Pinch { ids: [a.0, b.0], start: [a.1, b.1], zoom: state.zoom, pan: state.pan });
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
                state.zoom = (pinch.zoom * ratio).clamp(MIN_ZOOM, MAX_ZOOM);
                state.pan = (pinch.pan.0 + dx, pinch.pan.1 + dy);
            }
            true
        }
        Phase::Up | Phase::Cancel => {
            state.touches.retain(|(id, _)| *id != e.id);
            if state.pinch.take().is_some() {
                // Render again at the new zoom once the fingers lift.
                state.due = Some(Instant::now() + Duration::from_millis(120));
                return true;
            }
            false
        }
    }
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
    }
    edits.dirty = true;
    state.crop = false;
    state.pan = (0.0, 0.0);
    state.zoom = 1.0;
    schedule(hwnd, state, 0);
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn frames_fit_center_and_scale_live_between_renders() {
        let doc = Rect { x0: 0.0, y0: 100.0, x1: 1000.0, y1: 700.0 };
        let fit = frame_rect(doc, (500, 600), 1.0, 1.0, (0.0, 0.0));
        assert_eq!(fit, Rect { x0: 250.0, y0: 100.0, x1: 750.0, y1: 700.0 });
        let large = frame_rect(doc, (2000, 1200), 1.0, 1.0, (0.0, 0.0));
        assert_eq!((large.width(), large.height()), (1000.0, 600.0));
        let zoomed = frame_rect(doc, (2000, 1200), 2.0, 2.0, (10.0, -5.0));
        assert_eq!((zoomed.width(), zoomed.x0, zoomed.y0), (2000.0, -490.0, -205.0));
        let pinching = frame_rect(doc, (500, 600), 2.0, 1.0, (0.0, 0.0));
        assert_eq!(pinching.width(), 1000.0);
    }
}
