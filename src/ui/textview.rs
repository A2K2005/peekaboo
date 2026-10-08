//! Text files in Quick view: the first 2 MB, in a monospace font, wrapped at
//! the view width and scrolled with the wheel and the paging keys.
use super::{
    paint::Look,
    render::{dwrite, measure, Align},
    widgets::Rect,
};
use std::{io::Read, ops::Range, path::Path};
use windows::{
    core::{w, Result},
    Win32::{
        Graphics::DirectWrite::{
            IDWriteTextFormat, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_WORD_WRAPPING_NO_WRAP,
        },
        UI::Input::KeyboardAndMouse::{VIRTUAL_KEY, VK_END, VK_HOME, VK_NEXT, VK_PRIOR},
    },
};

const LIMIT: u64 = 2 << 20;
const SNIFF: u64 = 4096;
const FONT_SIZE: f32 = 13.0;
const PAD: f32 = 12.0;

/// Types shown as text even when their bytes are not UTF-8, and before any
/// preview handler.
const TYPES: &[&str] = &[
    "txt", "text", "md", "markdown", "csv", "tsv", "json", "jsonc", "log", "xml", "yaml", "yml", "toml", "ini", "cfg",
    "conf", "properties", "rs", "py", "js", "mjs", "cjs", "ts", "tsx", "jsx", "c", "h", "cpp", "hpp", "cc", "cs", "java",
    "kt", "go", "rb", "php", "swift", "sh", "bat", "cmd", "ps1", "psm1", "sql", "css", "scss", "less", "lua", "r", "pl",
    "dart", "scala", "vb", "fs", "tex", "rst", "srt", "vtt", "reg", "inf", "gradle", "vue", "svelte", "diff", "patch",
    "nfo",
];

/// True when `path` has a text type.
pub(super) fn known(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| TYPES.iter().any(|t| t.eq_ignore_ascii_case(e)))
}

/// The first 4 KB of `path`, or None when it cannot be read.
pub(super) fn head(path: &Path) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(SNIFF).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// True when `head` reads as text: a UTF-16 byte order mark, or no NUL bytes
/// and valid UTF-8. `lenient` accepts other 8-bit text too.
pub(super) fn is_text(head: &[u8], lenient: bool) -> bool {
    head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]) || (!head.contains(&0) && (lenient || utf8(head)))
}

/// A character cut at the end of a sample still counts as valid.
fn utf8(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).map_or_else(|e| e.error_len().is_none(), |_| true)
}

/// The first 2 MB of `path`, decoded, with tabs as 4 spaces and no CRs.
pub(super) fn read(path: &Path) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.take(LIMIT + 1).read_to_end(&mut bytes)?;
    let cut = bytes.len() as u64 > LIMIT;
    bytes.truncate(LIMIT as usize);
    let mut text = decode(&bytes);
    if cut {
        text.push_str("\n\nQuick view shows the first 2 MB of this file.");
    }
    Ok(text.replace('\r', "").replace('\t', "    "))
}

fn decode(bytes: &[u8]) -> String {
    let utf16 = |rest: &[u8], unit: fn([u8; 2]) -> u16| {
        String::from_utf16_lossy(&rest.chunks_exact(2).map(|c| unit([c[0], c[1]])).collect::<Vec<_>>())
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, u16::from_le_bytes),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, u16::from_be_bytes),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ if utf8(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        // ponytail: Latin-1, not the PC's code page, so bytes 0x80 to 0x9F (curly quotes, euro) come out wrong; MultiByteToWideChar(CP_ACP) fixes it.
        _ => bytes.iter().map(|&b| b as char).collect(),
    }
}

/// Byte ranges of the rows of `text` wrapped at `columns` characters, at the
/// last space when the row has one, else mid-word.
fn wrap(text: &str, columns: usize) -> Vec<Range<usize>> {
    let mut rows = Vec::new();
    let mut base = 0;
    for line in text.split('\n') {
        let mut start = 0;
        let mut count = 0;
        let mut space = None;
        for (at, c) in line.char_indices() {
            if count == columns {
                let end = space.filter(|&s| s > start).unwrap_or(at);
                rows.push(base + start..base + end);
                start = end;
                count = line[start..at].chars().count();
                space = None;
            }
            if c == ' ' {
                space = Some(at + 1);
            }
            count += 1;
        }
        rows.push(base + start..base + line.len());
        base += line.len() + 1;
    }
    rows
}

fn monospace(size: f32) -> Result<IDWriteTextFormat> {
    unsafe {
        let format = dwrite()?.CreateTextFormat(
            w!("Consolas"),
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("en-us"),
        )?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        Ok(format)
    }
}

pub(super) struct TextView {
    text: String,
    rows: Vec<Range<usize>>,
    columns: usize,
    /// Pixels from the top.
    scroll: f32,
    line: f32,
    view: f32,
    /// Font size, format, and the width of one character.
    font: Option<(f32, IDWriteTextFormat, f32)>,
}

impl TextView {
    pub(super) fn new(text: String) -> Self {
        Self { text, rows: Vec::new(), columns: 0, scroll: 0.0, line: 16.0, view: 0.0, font: None }
    }

    pub(super) fn wheel(&mut self, delta: f32) {
        self.scroll -= delta / 120.0 * 3.0 * self.line;
    }

    /// Page Up, Page Down, Home, and End. Returns true when handled.
    pub(super) fn key(&mut self, key: VIRTUAL_KEY) -> bool {
        let page = (self.view - self.line).max(self.line);
        match key {
            VK_PRIOR => self.scroll -= page,
            VK_NEXT => self.scroll += page,
            VK_HOME => self.scroll = 0.0,
            VK_END => self.scroll = f32::MAX,
            _ => return false,
        }
        true
    }

    /// Draws the rows in view on a page that fills `r`, and a thin scroll
    /// bar when the text is longer than the view.
    pub(super) fn paint(&mut self, l: &Look, r: Rect, text_scale: f32) {
        l.p.fill_round(r, 6.0 * l.s, l.t.field);
        let size = FONT_SIZE * l.s * text_scale;
        if self.font.as_ref().map(|f| f.0) != Some(size) {
            self.font = monospace(size).ok().map(|format| {
                let advance = measure(&"0".repeat(32), &format, 100_000.0).0 / 32.0;
                self.line = measure("Ag", &format, 100_000.0).1.max(1.0);
                (size, format, advance.max(1.0))
            });
            self.columns = 0;
        }
        let Some((_, format, advance)) = &self.font else {
            return;
        };
        let inner = r.inset(PAD * l.s);
        let columns = ((inner.width() / advance).floor() as usize).max(1);
        if columns != self.columns {
            self.rows = wrap(&self.text, columns);
            self.columns = columns;
        }
        self.view = inner.height();
        let total = self.rows.len() as f32 * self.line;
        let room = (total - self.view).max(0.0);
        self.scroll = self.scroll.clamp(0.0, room);
        l.p.push_clip(r);
        let first = (self.scroll / self.line) as usize;
        for (index, row) in self.rows.iter().enumerate().skip(first) {
            let y = inner.y0 + index as f32 * self.line - self.scroll;
            if y > r.y1 {
                break;
            }
            let rect = Rect { x0: inner.x0, y0: y, x1: inner.x1, y1: y + self.line };
            l.p.text(self.text.get(row.clone()).unwrap_or_default(), rect, format, l.t.text, Align::Leading);
        }
        if room > 0.0 {
            let height = (self.view / total * self.view).max(24.0 * l.s);
            let y = inner.y0 + self.scroll / room * (self.view - height);
            l.p.fill_round(Rect::new(r.x1 - 6.0 * l.s, y, 3.0 * l.s, height), 1.5 * l.s, l.t.text_secondary.alpha(0.5));
        }
        l.p.pop_clip();
    }
}
