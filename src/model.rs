pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub enum ImageEdit {
    RotateRight,
    FlipHorizontal,
    FlipVertical,
    Crop {
        left: f32,
        top: f32,
        right: f32,
        bottom: f32,
    },
    Resize {
        width: u32,
        height: u32,
    },
    /// A mark in the default style (`MarkStyle::default`).
    Annotate {
        kind: AnnotationKind,
        points: Vec<[f32; 2]>,
        text: String,
    },
    /// A mark with a chosen line, colors, and text style.
    Mark {
        kind: AnnotationKind,
        points: Vec<[f32; 2]>,
        text: String,
        style: MarkStyle,
    },
    /// Clears the rectangle, or the ellipse inside it; with `outside`,
    /// everything else. Cleared pixels turn transparent, or white when
    /// `white`, for formats that cannot store transparency.
    Clear {
        rect: NormRect,
        ellipse: bool,
        outside: bool,
        white: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum PdfEdit {
    RotateRight {
        page: u32,
    },
    Delete {
        page: u32,
    },
    /// Highlight, Underline, and Strikeout take `points` as corner pairs
    /// (top-left, bottom-right): one quad per pair, for text over several lines.
    Annotate {
        page: u32,
        kind: AnnotationKind,
        points: Vec<[f32; 2]>,
        text: String,
    },
    /// `Annotate` with a chosen style.
    Mark {
        page: u32,
        kind: AnnotationKind,
        points: Vec<[f32; 2]>,
        text: String,
        style: MarkStyle,
    },
    FillField {
        page: u32,
        annotation_index: u32,
        value: String,
    },
    Move {
        from: u32,
        to: u32,
    },
    InsertBlank {
        at: u32,
    },
    Crop {
        page: u32,
        left: f32,
        top: f32,
        right: f32,
        bottom: f32,
    },
    /// Removes the annotation at `index` in the page's annotation list.
    DeleteAnnotation {
        page: u32,
        index: u32,
    },
    /// Replaces the annotation's text (its Contents entry).
    SetAnnotationText {
        page: u32,
        index: u32,
        text: String,
    },
    /// Inserts an image as a new page at `at`. The page takes the size of
    /// its neighbor page, turned to match the image orientation. `bytes` is
    /// the image file as read at insert time, so the recipe does not change
    /// when the file moves or changes. `name` (the file name) is for display.
    InsertImage {
        at: u32,
        name: String,
        bytes: std::sync::Arc<[u8]>,
    },
    /// Inserts every page of another PDF at `at`. As with `InsertImage`,
    /// `bytes` is the file as read at insert time and `name` is for display.
    InsertPdf {
        at: u32,
        name: String,
        bytes: std::sync::Arc<[u8]>,
    },
    /// A drawn signature placed in `rect`. Each stroke point is
    /// `[x, y, pressure]`, with x and y in 0..1 of `rect`.
    Sign {
        page: u32,
        rect: NormRect,
        strokes: Vec<Vec<[f32; 3]>>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AnnotationKind {
    Ink,
    Highlight,
    Underline,
    Strikeout,
    Note,
    Rectangle,
    Ellipse,
    Arrow,
    Text,
    Line,
    RoundedRectangle,
    Bubble,
    Star,
    Polygon,
    /// A round magnifier over the image. Images only.
    Loupe,
    /// Dims everything outside the box.
    Mask,
}

impl AnnotationKind {
    /// Shapes with an inside that a fill color paints.
    pub fn closed(self) -> bool {
        use AnnotationKind::*;
        matches!(self, Rectangle | Ellipse | RoundedRectangle | Bubble | Star | Polygon | Text)
    }
}

/// How a mark looks. Colors are `0xRRGGBBAA`; alpha 0 means none. `width`
/// (lines) and `size` (text) are points on a PDF page; images scale them
/// with the image (`imaging::annotated`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkStyle {
    pub width: f32,
    pub stroke: u32,
    pub fill: u32,
    pub text: u32,
    pub size: f32,
    pub font: MarkFont,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkFont {
    Sans,
    Serif,
    Mono,
}

impl Default for MarkStyle {
    fn default() -> Self {
        Self { width: 2.0, stroke: 0x0078D4FF, fill: 0, text: 0x0078D4FF, size: 12.0, font: MarkFont::Sans }
    }
}

impl MarkStyle {
    /// The style with a visible line where `kind` needs one: lines always,
    /// and closed shapes when they have no fill either.
    pub fn visible(self, kind: AnnotationKind) -> Self {
        let none = |color: u32| color & 0xFF == 0;
        if none(self.stroke) && (!kind.closed() || none(self.fill)) && kind != AnnotationKind::Text {
            return Self { stroke: Self::default().stroke, ..self };
        }
        self
    }
}

/// A `0xRRGGBBAA` color as red, green, blue, and alpha from 0 to 1.
pub fn rgba(color: u32) -> [f32; 4] {
    color.to_be_bytes().map(|c| c as f32 / 255.0)
}

/// The outline of a rounded rectangle, speech bubble, star, or polygon in
/// the box from `a` to `b`, as a closed polygon in the same normalized
/// coordinates. `aspect` is the page or image width over its height, so
/// corners stay round. None for other kinds.
pub fn outline(kind: AnnotationKind, a: [f32; 2], b: [f32; 2], aspect: f32) -> Option<Vec<[f32; 2]>> {
    use std::f32::consts::{FRAC_PI_2, TAU};
    use AnnotationKind::*;
    let (l, t, r, bottom) = (a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1]));
    let (w, h) = (r - l, bottom - t);
    let ring = |count: usize, inner: f32| -> Vec<[f32; 2]> {
        (0..count)
            .map(|i| {
                let angle = TAU * i as f32 / count as f32 - FRAC_PI_2;
                let k = if i % 2 == 1 && inner > 0.0 { inner } else { 1.0 };
                [l + w / 2.0 * (1.0 + k * angle.cos()), t + h / 2.0 * (1.0 + k * angle.sin())]
            })
            .collect()
    };
    // Corner arcs of a rounded rectangle from `t` down to `low`. A tail,
    // when given, goes on the bottom edge between the last two corners.
    let rounded = |low: f32, tail: &[[f32; 2]]| -> Vec<[f32; 2]> {
        let aspect = aspect.max(1e-3);
        let rx = 0.15 * w.min((low - t) / aspect);
        let ry = rx * aspect;
        let corners = [([r - rx, t + ry], -90.0), ([r - rx, low - ry], 0.0), ([l + rx, low - ry], 90.0), ([l + rx, t + ry], 180.0)];
        let mut points = Vec::new();
        for (index, (center, from)) in corners.into_iter().enumerate() {
            if index == 2 {
                points.extend_from_slice(tail);
            }
            for step in 0..=6 {
                let angle = (from + 15.0 * step as f32).to_radians();
                points.push([center[0] + angle.cos() * rx, center[1] + angle.sin() * ry]);
            }
        }
        points
    };
    Some(match kind {
        RoundedRectangle => rounded(bottom, &[]),
        Bubble => {
            let body = t + 0.75 * h;
            rounded(body, &[[l + 0.4 * w, body], [l + 0.15 * w, bottom], [l + 0.25 * w, body]])
        }
        Star => ring(10, 0.4),
        Polygon => ring(6, 0.0),
        _ => return None,
    })
}

/// A rectangle normalized to the page or image: `[left, top, right, bottom]`,
/// each in 0..1, with the origin at the top-left corner.
pub type NormRect = [f32; 4];

/// Selectable text for one PDF page or one image, in reading order.
/// PDF text and OCR text both use this type, so selection, copy, and
/// search highlighting work the same way for both.
/// Invariant: `boxes.len() == text.chars().count()`. Line breaks and
/// inserted spaces have zero-size boxes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextLayer {
    pub text: String,
    pub boxes: Vec<NormRect>,
}

/// One search match. A match that wraps across lines has several rects.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchHit {
    pub page: u32,
    pub rects: Vec<NormRect>,
}

/// One table-of-contents entry. `level` is 0 for top-level entries.
/// `page` is `None` when the entry has no destination in this document.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineItem {
    pub title: String,
    pub page: Option<u32>,
    pub level: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PdfFormType {
    None,
    AcroForm,
    XfaFull,
    XfaForeground,
}

/// Document information. Dates are raw PDF date strings, such as
/// `D:20261006120000Z`. `version` is like `1.7`.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfMetadata {
    pub title: String,
    pub author: String,
    pub subject: String,
    pub keywords: String,
    pub creator: String,
    pub producer: String,
    pub created: String,
    pub modified: String,
    pub version: String,
    pub page_count: u32,
    pub encrypted: bool,
    pub form: PdfFormType,
}

/// An annotation on a page. `kind` is the PDF subtype name, such as
/// `Highlight`, `Ink`, `Link`, or `Widget`. `index` is the position in the
/// page's annotation list, as used by `PdfEdit::DeleteAnnotation`.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfAnnotation {
    pub index: u32,
    pub kind: String,
    pub rect: NormRect,
    pub contents: String,
}

/// Input for the inline form session. Coordinates are points from the
/// top-left corner of the displayed page, in the units of `page_sizes`.
/// Send WM_KEYDOWN keys as `Key` (virtual-key codes) and WM_CHAR text as
/// `Char`. Tab goes only through `Key`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FormInput {
    PointerDown {
        x: f32,
        y: f32,
    },
    PointerUp {
        x: f32,
        y: f32,
    },
    PointerMove {
        x: f32,
        y: f32,
    },
    Char(char),
    Key {
        code: u32,
        shift: bool,
        ctrl: bool,
        alt: bool,
    },
    Blur,
    /// Focuses the field with this annotation index on the page.
    Focus(u32),
}

/// Result of one form input. `focus` is the page and rectangle of the
/// focused field. Append `commits` to the edit recipe in order: the open
/// document already holds those values, so the session stays intact.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FormFeedback {
    pub focus: Option<(u32, NormRect)>,
    pub redraw: bool,
    pub commits: Vec<PdfEdit>,
}

/// How `save_incremental` wrote the file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SaveMode {
    /// The original bytes, unchanged, plus an appended update.
    Incremental,
    /// A complete rewrite, because the incremental update did not verify.
    Full,
}

#[derive(Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub page_count: u32,
    pub source_width: u32,
    pub source_height: u32,
}

/// Image file formats the app can write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Jpeg,
    Png,
    WebP,
    Tiff,
    Heic,
    Bmp,
}

/// `quality` is 0..1 and applies to JPEG, HEIC, and lossy WebP.
/// `lossless` applies to WebP only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExportOptions {
    pub format: ImageFormat,
    pub quality: f32,
    pub lossless: bool,
}

/// How a batch resizes each image. `Percent` is 100 for the original size.
/// `MaxEdge` shrinks the longest side to the value and never enlarges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BatchResize {
    Pixels { width: u32, height: u32 },
    Percent(f32),
    MaxEdge(u32),
}

/// One batch action, applied to every input: rotate, then resize, then
/// write in `options.format` (or each input's own format when `None`).
#[derive(Clone, Debug, PartialEq)]
pub struct BatchJob {
    pub quarter_turns: u8,
    pub resize: Option<BatchResize>,
    pub options: Option<ExportOptions>,
}

/// The outcome for one batch input: the new file, or why it failed.
#[derive(Clone, Debug, PartialEq)]
pub struct BatchResult {
    pub input: std::path::PathBuf,
    pub output: Result<std::path::PathBuf, String>,
}

pub fn frame_bytes(width: u32, height: u32) -> Result<usize, String> {
    let len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .filter(|n| *n > 0 && *n <= MAX_FRAME_BYTES)
        .ok_or("The image is too large to display safely.")?;
    Ok(len)
}

pub fn fit_size(
    width: u32,
    height: u32,
    max_width: u32,
    max_height: u32,
) -> Result<(u32, u32), String> {
    if width == 0 || height == 0 || max_width == 0 || max_height == 0 {
        return Err("The file has invalid dimensions.".into());
    }
    let scale = (max_width as f64 / width as f64)
        .min(max_height as f64 / height as f64)
        .min(1.0);
    let size = (
        ((width as f64 * scale).round() as u32).max(1),
        ((height as f64 * scale).round() as u32).max(1),
    );
    frame_bytes(size.0, size.1)?;
    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_overflow_and_zero_dimensions() {
        assert!(frame_bytes(u32::MAX, u32::MAX).is_err());
        assert!(frame_bytes(0, 100).is_err());
        assert!(fit_size(0, 20, 50, 50).is_err());
        assert!(fit_size(20, 20, 0, 50).is_err());
    }
    #[test]
    fn fits_without_stretching_or_zero_edges() {
        assert_eq!(fit_size(6000, 4000, 1200, 800).unwrap(), (1200, 800));
        assert_eq!(fit_size(30, 20, 1200, 800).unwrap(), (30, 20));
        assert_eq!(fit_size(1, 1_000_000, 100, 100).unwrap(), (1, 100));
    }
}
