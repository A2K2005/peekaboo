pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub enum ImageEdit {
    RotateRight,
    FlipHorizontal,
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
    Annotate {
        kind: AnnotationKind,
        points: Vec<[f32; 2]>,
        text: String,
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
    Annotate {
        page: u32,
        kind: AnnotationKind,
        points: Vec<[f32; 2]>,
        text: String,
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
    // PDF types (W1-B). The shell starts using these in wave 2.
    /// Removes the annotation at `index` in the page's annotation list.
    #[allow(dead_code)]
    DeleteAnnotation {
        page: u32,
        index: u32,
    },
    /// Replaces the annotation's text (its Contents entry).
    #[allow(dead_code)]
    SetAnnotationText {
        page: u32,
        index: u32,
        text: String,
    },
    /// Inserts an image as a new page at `at`. The page takes the size of
    /// its neighbor page, turned to match the image orientation. `bytes` is
    /// the image file as read at insert time, so the recipe does not change
    /// when the file moves or changes. `name` (the file name) is for display.
    #[allow(dead_code)]
    InsertImage {
        at: u32,
        name: String,
        bytes: std::sync::Arc<[u8]>,
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

// PDF types (W1-B)

/// One table-of-contents entry. `level` is 0 for top-level entries.
/// `page` is `None` when the entry has no destination in this document.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineItem {
    pub title: String,
    pub page: Option<u32>,
    pub level: u32,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PdfFormType {
    None,
    AcroForm,
    XfaFull,
    XfaForeground,
}

/// Document information. Dates are raw PDF date strings, such as
/// `D:20261006120000Z`. `version` is like `1.7`.
#[allow(dead_code)]
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
#[allow(dead_code)]
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
#[allow(dead_code)]
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
}

/// Result of one form input. `focus` is the page and rectangle of the
/// focused field. Append `commits` to the edit recipe in order: the open
/// document already holds those values, so the session stays intact.
#[allow(dead_code)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FormFeedback {
    pub focus: Option<(u32, NormRect)>,
    pub redraw: bool,
    pub commits: Vec<PdfEdit>,
}

/// How `save_incremental` wrote the file.
#[allow(dead_code)]
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

// Imaging types (W1-C)

/// Image file formats the app can write.
#[allow(dead_code)]
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
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExportOptions {
    pub format: ImageFormat,
    pub quality: f32,
    pub lossless: bool,
}

/// How a batch resizes each image. `Percent` is 100 for the original size.
/// `MaxEdge` shrinks the longest side to the value and never enlarges.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BatchResize {
    Pixels { width: u32, height: u32 },
    Percent(f32),
    MaxEdge(u32),
}

/// One batch action, applied to every input: rotate, then resize, then
/// write in `options.format` (or each input's own format when `None`).
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub struct BatchJob {
    pub quarter_turns: u8,
    pub resize: Option<BatchResize>,
    pub options: Option<ExportOptions>,
}

/// The outcome for one batch input: the new file, or why it failed.
#[allow(dead_code)]
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
