//! Text selection, copy, and in-image search over `TextLayer`s (PRD tasks 2
//! and 9). PDF pages (PDFium) and images (Windows OCR) give the same layer,
//! so one set of pure functions serves both. Positions are char indices in
//! a layer's text; `TextLayer` keeps one box per char.
//!
//! Status (W2-2): this module is the tested core. The shell wiring (pointer
//! and I-beam, find bar, OCR on hover, accessibility text runs) is not done.
use crate::model::{NormRect, SearchHit, TextLayer};
use std::{borrow::Borrow, ops::Range};

/// A caret: a page and a char index in its layer, 0..=len. Images are page 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Pos {
    pub(super) page: u32,
    pub(super) index: usize,
}

/// Past the end of any page, for spans whose layer is not loaded yet.
pub(super) const END: usize = usize::MAX;

/// A selection in reading order. `anchor` stays put; `focus` follows the
/// pointer or Shift+click. The markup slice turns `range` and `rects` into
/// highlight, underline, or strikeout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Selection {
    pub(super) anchor: Pos,
    pub(super) focus: Pos,
}

impl Selection {
    pub(super) fn caret(at: Pos) -> Self {
        Self { anchor: at, focus: at }
    }
    /// Start and end in reading order.
    pub(super) fn span(&self) -> (Pos, Pos) {
        (self.anchor.min(self.focus), self.anchor.max(self.focus))
    }
    pub(super) fn is_empty(&self) -> bool {
        self.anchor == self.focus
    }
    /// The chars selected on `page`, given that page's char count.
    pub(super) fn range(&self, page: u32, len: usize) -> Option<Range<usize>> {
        let (start, end) = self.span();
        if page < start.page || page > end.page {
            return None;
        }
        let from = if page == start.page { start.index.min(len) } else { 0 };
        let to = if page == end.page { end.index.min(len) } else { len };
        (from < to).then_some(from..to)
    }
}

/// Ctrl+A: every char of every page.
pub(super) fn all(pages: u32) -> Selection {
    Selection { anchor: Pos { page: 0, index: 0 }, focus: Pos { page: pages.saturating_sub(1), index: END } }
}

fn is_box(b: &NormRect) -> bool {
    b[2] > b[0] && b[3] > b[1]
}

fn gap(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo - v
    } else if v > hi {
        v - hi
    } else {
        0.0
    }
}

/// How much more a vertical miss counts than a horizontal one, so a point
/// right of a short line picks that line, not a longer line below.
/// ponytail: one fixed weight; tune it if multi-column pages pick a wrong column.
const VERTICAL: f32 = 100.0;

/// The char under or nearest to `point` (page fractions). `size` is the
/// page in pixels, so distances are isotropic. Zero-size boxes (inserted
/// spaces and line breaks) never match.
fn nearest(layer: &TextLayer, point: [f32; 2], size: [f32; 2]) -> Option<usize> {
    let (x, y) = (point[0] * size[0], point[1] * size[1]);
    layer
        .boxes
        .iter()
        .enumerate()
        .filter(|(_, b)| is_box(b))
        .map(|(i, b)| (i, gap(x, b[0] * size[0], b[2] * size[0]) + VERTICAL * gap(y, b[1] * size[1], b[3] * size[1])))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// The caret for a press at `point`: before or after the nearest char.
pub(super) fn caret(layer: &TextLayer, point: [f32; 2], size: [f32; 2]) -> Option<usize> {
    let i = nearest(layer, point, size)?;
    let b = layer.boxes[i];
    Some(if point[0] > (b[0] + b[2]) / 2.0 { i + 1 } else { i })
}

/// True when `point` is on a line of text, for the I-beam. Boxes widen by
/// half their height on each side, so word gaps keep the I-beam.
pub(super) fn over_text(layer: &TextLayer, point: [f32; 2], size: [f32; 2]) -> bool {
    layer.boxes.iter().filter(|b| is_box(b)).any(|b| {
        let pad = (b[3] - b[1]) * size[1] / 2.0 / size[0].max(1.0);
        point[0] >= b[0] - pad && point[0] <= b[2] + pad && point[1] >= b[1] && point[1] <= b[3]
    })
}

fn class(c: char) -> u8 {
    if c.is_whitespace() {
        0
    } else if c.is_alphanumeric() || c == '_' {
        1
    } else {
        2
    }
}

/// Double-click: the run of letters and digits, of spaces, or of other
/// marks around char `index`.
pub(super) fn word(layer: &TextLayer, index: usize) -> Range<usize> {
    let chars: Vec<char> = layer.text.chars().collect();
    if index >= chars.len() {
        return chars.len()..chars.len();
    }
    let kind = class(chars[index]);
    let start = chars[..index].iter().rposition(|c| class(*c) != kind).map_or(0, |i| i + 1);
    let end = chars[index..].iter().position(|c| class(*c) != kind).map_or(chars.len(), |i| index + i);
    start..end
}

/// Triple-click: the line around char `index`, without its line break.
pub(super) fn line(layer: &TextLayer, index: usize) -> Range<usize> {
    let chars: Vec<char> = layer.text.chars().collect();
    let index = index.min(chars.len());
    let start = chars[..index].iter().rposition(|c| *c == '\n').map_or(0, |i| i + 1);
    let end = chars[index..].iter().position(|c| *c == '\n').map_or(chars.len(), |i| index + i);
    start..end
}

/// Lines as char ranges, each with its trailing line break, as AccessKit
/// text runs need them.
pub(super) fn lines(layer: &TextLayer) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, c) in layer.text.chars().enumerate() {
        if c == '\n' {
            out.push(start..i + 1);
            start = i + 1;
        }
    }
    if start < layer.boxes.len() {
        out.push(start..layer.boxes.len());
    }
    out
}

/// Highlight rectangles for chars in `range`: one per line piece.
pub(super) fn rects(layer: &TextLayer, range: Range<usize>) -> Vec<NormRect> {
    let end = range.end.min(layer.boxes.len());
    let mut out: Vec<NormRect> = Vec::new();
    for b in layer.boxes[range.start.min(end)..end].iter().filter(|b| is_box(b)) {
        match out.last_mut() {
            Some(r) if r[3].min(b[3]) - r[1].max(b[1]) > 0.5 * (r[3] - r[1]).min(b[3] - b[1]) => {
                *r = [r[0].min(b[0]), r[1].min(b[1]), r[2].max(b[2]), r[3].max(b[3])];
            }
            _ => out.push(*b),
        }
    }
    out
}

/// The selected text in reading order, pages joined by a line break.
/// `layer` gives a page's layer: the document worker passes PDFium's (after
/// checking `can_copy`), the window passes its cached OCR layer.
pub(super) fn selected_text<L: Borrow<TextLayer>>(
    selection: &Selection,
    mut layer: impl FnMut(u32) -> Result<L, String>,
) -> Result<String, String> {
    let (start, end) = selection.span();
    let mut pages = Vec::new();
    for page in start.page..=end.page {
        let layer = layer(page)?;
        let layer = layer.borrow();
        if let Some(range) = selection.range(page, layer.boxes.len()) {
            pages.push(layer.text.chars().skip(range.start).take(range.len()).collect::<String>());
        }
    }
    Ok(pages.join("\n"))
}

/// Windows editors expect CRLF line breaks on the clipboard.
pub(super) fn clipboard_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// Every match of `query` in one layer. Images search their OCR text with
/// this; PDFs use PDFium's search on the document worker.
pub(super) fn find(layer: &TextLayer, page: u32, query: &str, match_case: bool) -> Vec<SearchHit> {
    let fold = |c: char| if match_case { c } else { c.to_lowercase().next().unwrap_or(c) };
    let hay: Vec<char> = layer.text.chars().map(fold).collect();
    let needle: Vec<char> = query.chars().map(fold).collect();
    let mut hits = Vec::new();
    let mut i = 0;
    while !needle.is_empty() && i + needle.len() <= hay.len() {
        if hay[i..i + needle.len()] == needle[..] {
            hits.push(SearchHit { page, rects: rects(layer, i..i + needle.len()) });
            i += needle.len();
        } else {
            i += 1;
        }
    }
    hits
}

/// Presses in a row: one within `time_ms` and `slop` pixels of the last
/// counts up to 3 (word, then line), then starts over. `last` is (time in
/// ms, point, count).
pub(super) fn clicks(last: Option<(u64, (f32, f32), u8)>, now_ms: u64, at: (f32, f32), time_ms: u64, slop: f32) -> u8 {
    match last {
        Some((t, p, n)) if now_ms.saturating_sub(t) <= time_ms && (at.0 - p.0).abs() <= slop && (at.1 - p.1).abs() <= slop => n % 3 + 1,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LETTER: [f32; 2] = [612.0, 792.0];

    /// Lines 0.1 apart, chars 0.02 wide from x = 0.1. Spaces and line
    /// breaks get zero-width boxes, as OCR gives them.
    fn layer(lines: &[&str]) -> TextLayer {
        let mut out = TextLayer::default();
        for (row, line) in lines.iter().enumerate() {
            let top = 0.1 * (row + 1) as f32;
            if row > 0 {
                let x = out.boxes.last().unwrap()[2];
                out.text.push('\n');
                out.boxes.push([x, top - 0.1, x, top - 0.08]);
            }
            for (col, c) in line.chars().enumerate() {
                let x = 0.1 + 0.02 * col as f32;
                out.text.push(c);
                out.boxes.push(if c == ' ' { [x, top, x, top + 0.02] } else { [x, top, x + 0.02, top + 0.02] });
            }
        }
        out
    }

    fn near(a: &[NormRect], b: &[NormRect]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4))
    }

    #[test]
    fn caret_lands_before_or_after_the_nearest_char() {
        let l = layer(&["Hello world", "Second line"]);
        assert_eq!(caret(&l, [0.123, 0.11], LETTER), Some(1), "left half of e");
        assert_eq!(caret(&l, [0.137, 0.11], LETTER), Some(2), "right half of e");
        assert_eq!(caret(&l, [0.9, 0.11], LETTER), Some(11), "right of a short line: its end, not the line below");
        assert_eq!(caret(&l, [0.145, 0.9], LETTER), Some(14), "below the text: the last line");
        assert_eq!(caret(&TextLayer::default(), [0.5, 0.5], LETTER), None);
    }

    #[test]
    fn i_beam_shows_on_lines_and_word_gaps_only() {
        let l = layer(&["Hello world", "Second line"]);
        assert!(over_text(&l, [0.11, 0.11], LETTER));
        assert!(over_text(&l, [0.21, 0.11], LETTER), "the gap between words");
        assert!(!over_text(&l, [0.11, 0.15], LETTER), "between lines");
        assert!(!over_text(&l, [0.9, 0.11], LETTER), "past the line end");
    }

    #[test]
    fn double_and_triple_click_select_a_word_and_a_line() {
        let l = layer(&["Hello world, again", "Second line"]);
        assert_eq!(word(&l, 7), 6..11, "world");
        assert_eq!(word(&l, 5), 5..6, "a space");
        assert_eq!(word(&l, 11), 11..12, "the comma alone");
        assert_eq!(line(&l, 7), 0..18);
        assert_eq!(line(&l, 21), 19..30);
        assert_eq!(word(&l, 99), 30..30);
        assert_eq!(lines(&l), vec![0..19, 19..30], "each line keeps its break");
    }

    #[test]
    fn selection_spans_pages_in_reading_order_and_copies_their_text() {
        let pages = [layer(&["Hello world", "Second line"]), layer(&["Third"])];
        let get = |p: u32| -> Result<&TextLayer, String> { pages.get(p as usize).ok_or("no page".to_string()) };
        // Dragged backward, from page 2 up to page 1.
        let s = Selection { anchor: Pos { page: 1, index: 3 }, focus: Pos { page: 0, index: 6 } };
        assert_eq!(s.range(0, 23), Some(6..23));
        assert_eq!(s.range(1, 5), Some(0..3));
        assert_eq!(s.range(2, 5), None);
        assert_eq!(selected_text(&s, get).unwrap(), "world\nSecond line\nThi");
        assert_eq!(selected_text(&all(2), get).unwrap(), "Hello world\nSecond line\nThird");
        assert!(Selection::caret(Pos { page: 0, index: 4 }).is_empty());
        assert_eq!(selected_text(&Selection::caret(Pos { page: 0, index: 4 }), get).unwrap(), "");
        assert!(selected_text(&all(3), get).is_err(), "a page that cannot be read fails the copy");
        assert_eq!(clipboard_text("a\nb\r\nc"), "a\r\nb\r\nc");
    }

    #[test]
    fn highlight_is_one_rect_per_line_piece() {
        let l = layer(&["Hello world", "Second line"]);
        assert!(near(&rects(&l, 3..15), &[[0.16, 0.1, 0.32, 0.12], [0.1, 0.2, 0.16, 0.22]]), "{:?}", rects(&l, 3..15));
        assert!(rects(&l, 5..6).is_empty(), "a lone space has no box");
        assert!(rects(&l, 20..999).len() == 1, "ranges past the end are cut");
    }

    #[test]
    fn image_search_folds_case_unless_asked() {
        let l = layer(&["Hello world", "Second line"]);
        let hits = find(&l, 0, "LINE", false);
        assert_eq!(hits.len(), 1);
        assert!(near(&hits[0].rects, &[[0.24, 0.2, 0.32, 0.22]]));
        assert!(find(&l, 0, "LINE", true).is_empty());
        assert_eq!(find(&l, 3, "o", false).len(), 3);
        assert_eq!(find(&l, 3, "o", false)[0].page, 3);
        assert!(find(&l, 0, "", false).is_empty());
    }

    #[test]
    fn clicks_count_to_three_then_start_over() {
        assert_eq!(clicks(None, 0, (10.0, 10.0), 500, 4.0), 1);
        assert_eq!(clicks(Some((0, (10.0, 10.0), 1)), 300, (12.0, 11.0), 500, 4.0), 2);
        assert_eq!(clicks(Some((0, (10.0, 10.0), 2)), 300, (10.0, 10.0), 500, 4.0), 3);
        assert_eq!(clicks(Some((0, (10.0, 10.0), 3)), 300, (10.0, 10.0), 500, 4.0), 1);
        assert_eq!(clicks(Some((0, (10.0, 10.0), 1)), 600, (10.0, 10.0), 500, 4.0), 1, "too slow");
        assert_eq!(clicks(Some((0, (10.0, 10.0), 1)), 100, (20.0, 10.0), 500, 4.0), 1, "too far");
    }
}
