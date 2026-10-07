// Text layer in reading order, and search.
// APIs: fpdf_text.h (FPDFText_GetUnicode, FPDFText_IsGenerated,
// FPDFText_GetLooseCharBox, FPDFText_FindStart/FindNext/GetSchResultIndex/
// GetSchCount/FindClose, FPDFText_CountRects/GetRect).
use super::*;
use crate::model::{SearchHit, TextLayer};
use std::{cmp::Reverse, collections::BinaryHeap};

/// One character from PDFium in content-stream order. `rect` is
/// `[left, top, right, bottom]` in displayed page points; `None` marks a
/// space PDFium inserted where the content has a gap but no space glyph.
#[derive(Clone, Copy, Debug)]
pub(super) struct Glyph {
    pub ch: char,
    pub rect: Option<[f32; 4]>,
}

#[allow(dead_code)]
impl PdfEngine {
    /// The page's text in reading order with one box per character, for
    /// selection, search highlights, and Narrator. It needs only the
    /// accessibility permission, which a PDF can grant while it forbids
    /// copying. Before text goes to the clipboard, check `can_copy`.
    pub fn text_layer(
        &mut self,
        path: &Path,
        page: u32,
        edits: &[PdfEdit],
    ) -> Result<TextLayer, String> {
        self.ensure(path, edits)?;
        let handle = self
            .document
            .as_ref()
            .ok_or("No PDF is open.")?
            .native
            .handle;
        // Copy is bit 5 (16). Accessibility is bit 10 (512) from security
        // handler revision 3; revision 2 uses the copy bit (ISO 32000-1, table 22).
        let permissions = unsafe { (self.api.permissions)(handle) };
        let revision = unsafe { (self.api.security_revision)(handle) };
        if permissions & 16 == 0 && (revision < 3 || permissions & 512 == 0) {
            return Err("This PDF's permissions do not allow reading its text.".into());
        }
        let page = self.api.page(handle, page)?;
        let display = self.api.display(page.handle)?;
        let text = self.api.text_page(page.handle)?;
        let glyphs = self.api.glyphs(text.handle, &display)?;
        Ok(layout(&glyphs, display.width as f32, display.height as f32))
    }

    /// True when the PDF allows copying its text. Copy, export, and
    /// clipboard paths need this; `text_layer` and `search` do not.
    pub fn can_copy(&mut self, path: &Path, edits: &[PdfEdit]) -> Result<bool, String> {
        self.ensure(path, edits)?;
        let handle = self
            .document
            .as_ref()
            .ok_or("No PDF is open.")?
            .native
            .handle;
        Ok(self.allow_copy(handle).is_ok())
    }

    /// Every match of `query`, page by page. Stops with `SEARCH_CANCELED`
    /// when `cancel` becomes true. Matches positions only, so it also works
    /// when the PDF forbids copying.
    pub fn search(
        &mut self,
        path: &Path,
        query: &str,
        match_case: bool,
        cancel: &AtomicBool,
        edits: &[PdfEdit],
    ) -> Result<Vec<SearchHit>, String> {
        if query.is_empty() || query.contains('\0') {
            return Err("Enter text to find.".into());
        }
        self.ensure(path, edits)?;
        let handle = self
            .document
            .as_ref()
            .ok_or("No PDF is open.")?
            .native
            .handle;
        let needle = wide(query);
        // FPDF_MATCHCASE = 1 (fpdf_text.h).
        let flags = match_case as u32;
        let count = unsafe { (self.api.count)(handle) }.max(0) as u32;
        let mut hits = Vec::new();
        for index in 0..count {
            if cancel.load(Ordering::Relaxed) {
                return Err(SEARCH_CANCELED.into());
            }
            let page = self.api.page(handle, index)?;
            let display = self.api.display(page.handle)?;
            let text = self.api.text_page(page.handle)?;
            unsafe {
                let find = (self.api.find_start)(text.handle, needle.as_ptr(), flags, 0);
                if find.is_null() {
                    continue;
                }
                let find = NativeHandle {
                    handle: find,
                    close: self.api.find_close,
                };
                while (self.api.find_next)(find.handle) != 0 {
                    let start = (self.api.find_index)(find.handle);
                    let length = (self.api.find_count)(find.handle);
                    let rects = (self.api.text_rects)(text.handle, start, length).max(0);
                    let rects = (0..rects)
                        .filter_map(|i| {
                            let (mut l, mut t, mut r, mut b) = (0.0, 0.0, 0.0, 0.0);
                            ((self.api.text_rect)(text.handle, i, &mut l, &mut t, &mut r, &mut b)
                                != 0)
                                .then(|| display.norm(display.rect(l, b, r, t)))
                        })
                        .collect();
                    hits.push(SearchHit { page: index, rects });
                }
            }
        }
        Ok(hits)
    }
}

impl Api {
    fn glyphs(&self, text: Handle, display: &Display) -> Result<Vec<Glyph>, String> {
        unsafe {
            let count = (self.text_count)(text);
            if !(0..=4_000_000).contains(&count) {
                return Err("The page contains too much text to read safely.".into());
            }
            let mut glyphs = Vec::with_capacity(count as usize);
            let mut index = 0;
            while index < count {
                let mut unit = (self.text_unicode)(text, index);
                let generated = (self.text_generated)(text, index) == 1;
                let mut rect = Rect::default();
                let boxed = (self.text_loose_box)(text, index, &mut rect) != 0;
                let mut bounds = display.rect(
                    rect.left as f64,
                    rect.bottom as f64,
                    rect.right as f64,
                    rect.top as f64,
                );
                index += 1;
                // Join a UTF-16 surrogate pair into one character and one box.
                if (0xD800..0xDC00).contains(&unit) && index < count {
                    let low = (self.text_unicode)(text, index);
                    if (0xDC00..0xE000).contains(&low) {
                        let mut other = Rect::default();
                        if (self.text_loose_box)(text, index, &mut other) != 0 {
                            let b = display.rect(
                                other.left as f64,
                                other.bottom as f64,
                                other.right as f64,
                                other.top as f64,
                            );
                            bounds = [
                                bounds[0].min(b[0]),
                                bounds[1].min(b[1]),
                                bounds[2].max(b[2]),
                                bounds[3].max(b[3]),
                            ];
                        }
                        unit = 0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00);
                        index += 1;
                    }
                }
                let ch = match char::from_u32(unit) {
                    Some('\t') => ' ',
                    Some(c) if !c.is_control() && unit != 0 => c,
                    _ => continue,
                };
                if generated {
                    // PDFium also generates line breaks; layout() makes its own.
                    if ch == ' ' {
                        glyphs.push(Glyph { ch, rect: None });
                    }
                } else if boxed && bounds.iter().all(|v| v.is_finite()) {
                    glyphs.push(Glyph {
                        ch,
                        rect: Some(bounds),
                    });
                }
            }
            Ok(glyphs)
        }
    }
}

/// A horizontal run of glyphs on one line. Box values are display points.
#[derive(Clone, Debug)]
struct Run {
    glyphs: Vec<usize>,
    l: f32,
    t: f32,
    r: f32,
    b: f32,
}
impl Run {
    fn height(&self) -> f32 {
        (self.b - self.t).max(0.01)
    }
    fn center(&self) -> f32 {
        (self.t + self.b) / 2.0
    }
}

fn vertical_overlap(t1: f32, b1: f32, t2: f32, b2: f32) -> f32 {
    b1.min(b2) - t1.max(t2)
}

fn same_row(a: &Run, b: &Run) -> bool {
    vertical_overlap(a.t, a.b, b.t, b.b) > 0.5 * a.height().min(b.height())
}

fn x_overlap(a: &Run, b: &Run) -> bool {
    a.r.min(b.r) - a.l.max(b.l) > 0.0
}

fn is_rtl(c: char) -> bool {
    matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF)
}

/// Builds the text layer. Glyphs form runs (content order, split where the
/// line changes, x jumps back, or a gap is wider than the line height). Runs
/// that follow each other one-to-one down a column form blocks. Blocks are
/// then sorted by a partial order based on Breuel (2003), "High Performance
/// Document Layout Analysis": above-before-below when blocks share x, and
/// left column before right column. Layout runs in the frame where the
/// text reads left to right, so rotated pages work too; boxes stay in
/// display coordinates.
pub(super) fn layout(glyphs: &[Glyph], width: f32, height: f32) -> TextLayer {
    let upright = upright(glyphs, width, height);
    let runs = runs(&upright);
    let mut text = String::new();
    let mut boxes = Vec::new();
    let norm = |r: [f32; 4]| -> NormRect {
        [
            (r[0] / width).clamp(0.0, 1.0),
            (r[1] / height).clamp(0.0, 1.0),
            (r[2] / width).clamp(0.0, 1.0),
            (r[3] / height).clamp(0.0, 1.0),
        ]
    };
    let mut last: Option<[f32; 4]> = None;
    let mut push =
        |text: &mut String, boxes: &mut Vec<NormRect>, ch: char, rect: Option<[f32; 4]>| {
            let rect = match rect {
                Some(rect) => {
                    last = Some(rect);
                    norm(rect)
                }
                // Inserted characters get a zero-size box at the previous glyph's end.
                None => last.map_or([0.0; 4], |r| {
                    let point = norm([r[2], r[1], r[2], r[1]]);
                    [point[0], point[1], point[0], point[1]]
                }),
            };
            text.push(ch);
            boxes.push(rect);
        };
    let mut previous: Option<&Run> = None;
    for index in reading_order(&runs) {
        let run = &runs[index];
        if let Some(prev) = previous {
            let separator = if same_row(prev, run) { ' ' } else { '\n' };
            let first = glyphs[run.glyphs[0]].ch;
            if !(separator == ' ' && (text.ends_with(' ') || first == ' ')) {
                push(&mut text, &mut boxes, separator, None);
            }
        }
        for &g in &run.glyphs {
            push(&mut text, &mut boxes, glyphs[g].ch, glyphs[g].rect);
        }
        previous = Some(run);
    }
    TextLayer { text, boxes }
}

/// The glyphs turned by a multiple of 90 degrees so that most neighbors in
/// content order advance along +x, as text does on an unrotated page.
fn upright(glyphs: &[Glyph], width: f32, height: f32) -> Vec<Glyph> {
    // Votes for the reading direction: +x, +y, -x, -y.
    let mut votes = [0usize; 4];
    let letters: Vec<[f32; 4]> = glyphs
        .iter()
        .filter(|g| !g.ch.is_whitespace())
        .filter_map(|g| g.rect)
        .collect();
    for pair in letters.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let dx = (b[0] + b[2] - a[0] - a[2]) / 2.0;
        let dy = (b[1] + b[3] - a[1] - a[3]) / 2.0;
        let size = (a[2] - a[0]).max(a[3] - a[1]);
        if dx.hypot(dy) <= 2.0 * size {
            let direction = match (dx.abs() >= dy.abs(), dx >= 0.0, dy >= 0.0) {
                (true, true, _) => 0,
                (false, _, true) => 1,
                (true, false, _) => 2,
                (false, _, false) => 3,
            };
            votes[direction] += 1;
        }
    }
    let direction = (0..4).max_by_key(|&d| (votes[d], d == 0)).unwrap_or(0);
    let turn = |x: f32, y: f32| match direction {
        1 => (y, width - x),
        2 => (width - x, height - y),
        3 => (height - y, x),
        _ => (x, y),
    };
    glyphs
        .iter()
        .map(|g| Glyph {
            ch: g.ch,
            rect: g.rect.map(|r| {
                let (x0, y0) = turn(r[0], r[1]);
                let (x1, y1) = turn(r[2], r[3]);
                [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)]
            }),
        })
        .collect()
}

fn runs(glyphs: &[Glyph]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let mut pending = Vec::new();
    // The last non-space glyph in the current run.
    let mut last: Option<(char, [f32; 4])> = None;
    for (index, glyph) in glyphs.iter().enumerate() {
        let Some(rect) = glyph.rect else {
            if last.is_some() {
                pending.push(index);
            }
            continue;
        };
        if glyph.ch.is_whitespace() {
            if let (Some(run), Some(_)) = (runs.last_mut(), last) {
                run.glyphs.append(&mut pending);
                run.glyphs.push(index);
            }
            continue;
        }
        let continues = match (runs.last(), last) {
            (Some(run), Some((prev_ch, prev))) => {
                let line = run.height();
                let glyph_height = (rect[3] - rect[1]).max(0.01);
                let overlap = vertical_overlap(run.t, run.b, rect[1], rect[3]);
                let rtl = is_rtl(prev_ch) && is_rtl(glyph.ch);
                let (forward, gap) = if rtl {
                    (rect[0] <= prev[0] + 0.25 * line, prev[0] - rect[2])
                } else {
                    (rect[0] >= prev[0] - 0.25 * line, rect[0] - prev[2])
                };
                overlap > 0.5 * line.min(glyph_height) && forward && gap <= line
            }
            _ => false,
        };
        if continues {
            let run = runs.last_mut().unwrap();
            run.glyphs.append(&mut pending);
            run.glyphs.push(index);
            run.l = run.l.min(rect[0]);
            run.t = run.t.min(rect[1]);
            run.r = run.r.max(rect[2]);
            run.b = run.b.max(rect[3]);
        } else {
            pending.clear();
            runs.push(Run {
                glyphs: vec![index],
                l: rect[0],
                t: rect[1],
                r: rect[2],
                b: rect[3],
            });
        }
        last = Some((glyph.ch, rect));
    }
    runs
}

/// Run indices in reading order.
fn reading_order(runs: &[Run]) -> Vec<usize> {
    let blocks = blocks(runs);
    // ponytail: pairwise ordering is cubic in the block count; pages with
    // more blocks fall back to rows. Add spatial indexing if that bites.
    let order = if blocks.len() > 400 {
        // "Same row" is not transitive, so it cannot drive a sort. Number
        // the rows first: top to bottom, a block starts a new row unless it
        // shares a line with the row's first block. Then sort by (row, left).
        let mut order: Vec<usize> = (0..blocks.len()).collect();
        order.sort_by(|&a, &b| blocks[a].center().total_cmp(&blocks[b].center()));
        let mut row = vec![0usize; blocks.len()];
        let mut first = order[0];
        for &i in &order {
            if !same_row(&blocks[first], &blocks[i]) {
                row[i] = row[first] + 1;
                first = i;
            } else {
                row[i] = row[first];
            }
        }
        order.sort_by(|&a, &b| {
            row[a]
                .cmp(&row[b])
                .then(blocks[a].l.total_cmp(&blocks[b].l))
        });
        order
    } else {
        topological(&blocks)
    };
    order
        .into_iter()
        .flat_map(|b| blocks[b].glyphs.clone())
        .collect()
}

/// Groups runs into blocks: a run joins the run above it when each is the
/// other's only close neighbor in that direction. Within a block, `glyphs`
/// holds run indices from top to bottom.
fn blocks(runs: &[Run]) -> Vec<Run> {
    let n = runs.len();
    // Vertical neighbor `b` of `a`: overlaps in x, starts below `a`, and the
    // gap is at most one line.
    let below = |a: &Run, b: &Run| {
        x_overlap(a, b)
            && b.t >= a.b - 0.25 * a.height().min(b.height())
            && b.t - a.b <= a.height().max(b.height())
    };
    let nearest = |candidates: Vec<usize>, key: &dyn Fn(usize) -> f32| -> Option<usize> {
        let best = candidates
            .iter()
            .map(|&c| key(c))
            .fold(f32::INFINITY, f32::min);
        let close: Vec<usize> = candidates
            .into_iter()
            .filter(|&c| key(c) <= best + 0.5 * runs[c].height())
            .collect();
        (close.len() == 1).then(|| close[0])
    };
    let mut down = vec![None; n];
    let mut up = vec![None; n];
    if n <= 4000 {
        for a in 0..n {
            let candidates = (0..n)
                .filter(|&b| b != a && below(&runs[a], &runs[b]))
                .collect();
            down[a] = nearest(candidates, &|b| runs[b].t);
            let candidates = (0..n)
                .filter(|&b| b != a && below(&runs[b], &runs[a]))
                .collect();
            up[a] = nearest(candidates, &|b| -runs[b].b);
        }
    }
    let mut blocks = Vec::new();
    for start in 0..n {
        // Start at runs that do not continue one above.
        if up[start].is_some_and(|u| down[u] == Some(start)) {
            continue;
        }
        let mut chain = vec![start];
        let mut at = start;
        while let Some(next) = down[at].filter(|&d| up[d] == Some(at)) {
            if chain.contains(&next) {
                break;
            }
            chain.push(next);
            at = next;
        }
        let pick = |f: fn(&Run) -> f32, min: bool| {
            chain.iter().map(|&i| f(&runs[i])).fold(
                if min {
                    f32::INFINITY
                } else {
                    f32::NEG_INFINITY
                },
                if min { f32::min } else { f32::max },
            )
        };
        blocks.push(Run {
            l: pick(|r| r.l, true),
            t: pick(|r| r.t, true),
            r: pick(|r| r.r, false),
            b: pick(|r| r.b, false),
            glyphs: chain,
        });
    }
    blocks
}

/// True when block `a` comes before block `b`, false when after, and `None`
/// when the pair has no direct order (it may still follow from others).
fn before(blocks: &[Run], a: usize, b: usize) -> Option<bool> {
    let (x, y) = (&blocks[a], &blocks[b]);
    if same_row(x, y) {
        return Some(x.l <= y.l);
    }
    if x_overlap(x, y) {
        return Some(x.center() < y.center());
    }
    let (left, right, a_is_left) = if x.r <= y.l {
        (x, y, true)
    } else {
        (y, x, false)
    };
    let (upper, lower) = if x.center() < y.center() {
        (x, y)
    } else {
        (y, x)
    };
    // A block between them in height that spans both columns separates them.
    let spans = |c: &Run| c.l < left.r && c.r > right.l;
    let between = |c: &Run| c.t >= upper.b - 0.01 && c.b <= lower.t + 0.01;
    if blocks.iter().any(|c| between(c) && spans(c)) {
        return None;
    }
    if left.center() > right.center() {
        // The left block is lower. It comes first only if its column has
        // text at the right block's height; otherwise the right block is a
        // heading or a date line above the text.
        let column = blocks
            .iter()
            .any(|c| x_overlap(c, left) && !x_overlap(c, right) && c.t <= right.b);
        if !column {
            return None;
        }
    }
    Some(a_is_left)
}

/// Kahn's topological sort over `before`, taking the topmost, then leftmost
/// free block first. A cycle is broken at its topmost block.
fn topological(blocks: &[Run]) -> Vec<usize> {
    let n = blocks.len();
    let mut next = vec![Vec::new(); n];
    let mut incoming = vec![0usize; n];
    for a in 0..n {
        for b in a + 1..n {
            match before(blocks, a, b) {
                Some(true) => {
                    next[a].push(b);
                    incoming[b] += 1;
                }
                Some(false) => {
                    next[b].push(a);
                    incoming[a] += 1;
                }
                None => {}
            }
        }
    }
    // f32 bit patterns of non-negative values sort like the values.
    let key = |i: usize| {
        Reverse((
            blocks[i].t.max(0.0).to_bits(),
            blocks[i].l.max(0.0).to_bits(),
            i,
        ))
    };
    let mut ready: BinaryHeap<_> = (0..n).filter(|&i| incoming[i] == 0).map(key).collect();
    let mut done = vec![false; n];
    let mut order = Vec::with_capacity(n);
    while order.len() < n {
        let index = match ready.pop() {
            Some(Reverse((_, _, i))) if done[i] => continue,
            Some(Reverse((_, _, i))) => i,
            None => match (0..n).filter(|&i| !done[i]).min_by_key(|&i| key(i).0) {
                Some(i) => i,
                None => break,
            },
        };
        done[index] = true;
        order.push(index);
        for &b in &next[index] {
            incoming[b] -= 1;
            if incoming[b] == 0 && !done[b] {
                ready.push(key(b));
            }
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Glyphs for `text` starting at (x, y) top-left in points, size 10.
    fn line(text: &str, x: f32, y: f32) -> Vec<Glyph> {
        text.chars()
            .enumerate()
            .map(|(i, ch)| Glyph {
                ch,
                rect: Some([x + i as f32 * 5.0, y, x + i as f32 * 5.0 + 5.0, y + 10.0]),
            })
            .collect()
    }

    fn text_of(lines: &[Vec<Glyph>]) -> String {
        layout(&lines.concat(), 600.0, 800.0).text
    }

    #[test]
    fn single_column_keeps_its_order() {
        let lines = [
            line("Title of the page", 50.0, 40.0),
            line("First line of body text.", 50.0, 70.0),
            line("Second line of body text.", 50.0, 82.0),
            line("Short end.", 50.0, 94.0),
            line("New paragraph here.", 70.0, 112.0),
        ];
        assert_eq!(
            text_of(&lines),
            "Title of the page\nFirst line of body text.\nSecond line of body text.\nShort end.\nNew paragraph here."
        );
    }

    #[test]
    fn two_columns_read_left_column_first_whatever_the_stream_order() {
        let left = ["L1 left", "L2 left", "L3 left"];
        let right = ["R1 right", "R2 right", "R3 right"];
        let mut lines = Vec::new();
        // The stream draws the right column first, bottom to top, then the left.
        for (i, t) in right.iter().enumerate().rev() {
            lines.push(line(t, 320.0, 100.0 + i as f32 * 12.0));
        }
        for (i, t) in left.iter().enumerate().rev() {
            lines.push(line(t, 50.0, 100.0 + i as f32 * 12.0));
        }
        // Glyphs are 5 points wide, so 70 characters span both columns.
        let heading = format!("{:=<70}", "Heading ");
        let footer = format!("{:-<70}", "Footer ");
        lines.insert(0, line(&footer, 50.0, 400.0));
        lines.push(line(&heading, 50.0, 60.0));
        assert_eq!(
            text_of(&lines),
            format!("{heading}\nL1 left\nL2 left\nL3 left\nR1 right\nR2 right\nR3 right\n{footer}")
        );
    }

    #[test]
    fn a_page_turned_clockwise_reads_like_the_upright_page() {
        let lines = [
            line("Title", 50.0, 40.0),
            line("Left one", 50.0, 100.0),
            line("Left two", 50.0, 112.0),
            line("Right one", 320.0, 100.0),
            line("Right two", 320.0, 112.0),
        ];
        // On a page shown turned clockwise, upright (x, y) displays at (800 - y, x).
        let turned: Vec<Glyph> = lines
            .concat()
            .into_iter()
            .map(|g| {
                let r = g.rect.unwrap();
                Glyph {
                    ch: g.ch,
                    rect: Some([800.0 - r[3], r[0], 800.0 - r[1], r[2]]),
                }
            })
            .collect();
        let layer = layout(&turned, 800.0, 600.0);
        assert_eq!(
            layer.text,
            "Title\nLeft one\nLeft two\nRight one\nRight two"
        );
        // Boxes stay in display coordinates: "T" is near the top right.
        assert!(
            layer.boxes[0][0] > 0.9 && layer.boxes[0][1] < 0.1,
            "{:?}",
            layer.boxes[0]
        );
    }

    #[test]
    fn rows_split_by_wide_gaps_stay_rows_and_spanning_text_separates_sections() {
        let full = format!("{:-<80}", "Full width line ");
        let caption = format!("{:-<80}", "Caption ");
        let lines = [
            line("Name", 50.0, 40.0),
            line("Date", 400.0, 40.0),
            line(&full, 50.0, 52.0),
            line("Top left", 50.0, 100.0),
            line("Top right", 320.0, 100.0),
            line(&caption, 50.0, 130.0),
            line("Bottom left", 50.0, 160.0),
            line("Bottom right", 320.0, 160.0),
        ];
        assert_eq!(
            text_of(&lines),
            format!("Name Date\n{full}\nTop left Top right\n{caption}\nBottom left Bottom right")
        );
    }

    #[test]
    fn a_date_line_above_on_the_right_stays_first() {
        let lines = [
            line("Dear reader,", 50.0, 100.0),
            line("October 7", 450.0, 40.0),
        ];
        assert_eq!(text_of(&lines), "October 7\nDear reader,");
    }

    #[test]
    fn a_dense_page_with_staggered_rows_reads_row_by_row() {
        // 600 one-letter blocks, each 4 points lower and 25 points further
        // left than the one before. Neighbors share a line, but blocks two
        // apart do not, so "same row, then left edge" is not a total order.
        let n = 600;
        let glyphs: Vec<Glyph> = (0..n)
            .map(|i| {
                let (x, y) = ((n - i) as f32 * 25.0, i as f32 * 4.0);
                Glyph {
                    ch: char::from(b'a' + (i % 26) as u8),
                    rect: Some([x, y, x + 5.0, y + 10.0]),
                }
            })
            .collect();
        let layer = layout(&glyphs, 20_000.0, 3_000.0);
        assert_eq!(layer.boxes.len(), layer.text.chars().count());
        // Rows pair up blocks (0, 1), (2, 3), ...; each row reads left to right.
        let lines: Vec<&str> = layer.text.lines().collect();
        assert_eq!(lines.len(), n / 2);
        assert_eq!(&lines[..3], ["b a", "d c", "f e"]);
    }

    #[test]
    fn inserted_characters_have_zero_size_boxes() {
        let mut glyphs = line("ab", 50.0, 40.0);
        glyphs.push(Glyph {
            ch: ' ',
            rect: None,
        });
        glyphs.extend(line("cd", 65.0, 40.0));
        glyphs.extend(line("ef", 50.0, 60.0));
        let layer = layout(&glyphs, 600.0, 800.0);
        assert_eq!(layer.text, "ab cd\nef");
        assert_eq!(layer.boxes.len(), layer.text.chars().count());
        for i in [2, 5] {
            let b = layer.boxes[i];
            assert!(b[0] == b[2] && b[1] == b[3], "box {i} must be empty: {b:?}");
        }
        assert!(layer.boxes[0][2] > layer.boxes[0][0]);
    }
}
