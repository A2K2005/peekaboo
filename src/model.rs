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
