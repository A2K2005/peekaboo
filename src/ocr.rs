use crate::model::{ImageEdit, NormRect, TextLayer};
use std::path::Path;
use windows::{
    Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap},
    Media::Ocr::{OcrEngine, OcrLine},
    Storage::Streams::DataWriter,
};

/// Call on the COM-initialized worker, never on the window thread.
pub fn recognize(path: &Path, edits: &[ImageEdit]) -> Result<String, String> {
    let layer = recognize_layer(path, edits)?;
    if layer.text.trim().is_empty() {
        return Err(
            "No text was recognized. Try a clearer image or check the installed OCR language."
                .into(),
        );
    }
    Ok(layer.text)
}

/// Recognize text with Windows.Media.Ocr in the user's profile languages.
/// Lines run top to bottom; each char gets a box normalized to the edited
/// image. Spaces and line breaks get zero-width boxes. An image with no
/// text gives an empty layer. Call on the COM-initialized worker.
/// https://learn.microsoft.com/uwp/api/windows.media.ocr.ocrengine
pub fn recognize_layer(path: &Path, edits: &[ImageEdit]) -> Result<TextLayer, String> {
    let maximum = OcrEngine::MaxImageDimension()
        .map_err(|e| format!("Windows text recognition is unavailable on this PC: {e}"))?
        .min(4096);
    if maximum == 0 {
        return Err("Windows text recognition reported an invalid image limit.".into());
    }
    let engine = engine()?;
    let mut frame = crate::imaging::decode_edited(path, maximum, maximum, edits)?;
    // OCR needs visible contrast: composite premultiplied pixels onto white paper.
    for pixel in frame.pixels.chunks_exact_mut(4) {
        let white = 255 - pixel[3];
        for channel in &mut pixel[..3] {
            *channel = channel.saturating_add(white);
        }
        pixel[3] = 255;
    }
    let bitmap = {
        let writer =
            DataWriter::new().map_err(|e| format!("Could not prepare text recognition: {e}"))?;
        writer
            .WriteBytes(&frame.pixels)
            .map_err(|e| format!("Could not prepare text recognition pixels: {e}"))?;
        let buffer = writer
            .DetachBuffer()
            .map_err(|e| format!("Could not prepare text recognition buffer: {e}"))?;
        SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
            &buffer,
            BitmapPixelFormat::Bgra8,
            frame.width as i32,
            frame.height as i32,
            BitmapAlphaMode::Premultiplied,
        )
        .map_err(|e| format!("Could not prepare the image for text recognition: {e}"))?
    };
    let (width, height) = (frame.width as f32, frame.height as f32);
    drop(frame);
    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(|e| format!("Windows could not start reading this image: {e}"))?
        .join()
        .map_err(|e| format!("Windows could not read text from this image: {e}"))?;
    let lines = result
        .Lines()
        .map_err(|e| format!("Windows could not return the recognized text: {e}"))?;
    let mut lines: Vec<(String, Vec<NormRect>)> = lines
        .into_iter()
        .map(|line| line_layer(&line, width, height))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("Windows could not return the recognized text: {e}"))?;
    lines.retain(|(text, _)| !text.is_empty());
    let mut layer = TextLayer::default();
    for (text, boxes) in reading_order(lines, width, height) {
        if let Some(last) = layer.boxes.last().copied() {
            layer.text.push('\n');
            layer.boxes.push([last[2], last[1], last[2], last[3]]);
        }
        layer.text.push_str(&text);
        layer.boxes.extend(boxes);
    }
    Ok(layer)
}

/// Lines in the engine's order: Windows already reads columns one at a
/// time. A line joins the previous row only when its middle falls inside
/// that row's height and the horizontal gap is under one line height, as
/// when Windows splits one line in two. A row reads left to right.
fn reading_order(
    lines: Vec<(String, Vec<NormRect>)>,
    width: f32,
    height: f32,
) -> Vec<(String, Vec<NormRect>)> {
    let bounds = |boxes: &[NormRect]| {
        boxes.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, r| {
            [b[0].min(r[0]), b[1].min(r[1]), b[2].max(r[2]), b[3].max(r[3])]
        })
    };
    let mut rows: Vec<(NormRect, Vec<(String, Vec<NormRect>)>)> = Vec::new();
    for line in lines {
        let b = bounds(&line.1);
        match rows.last_mut() {
            Some((r, row))
                if (r[1]..=r[3]).contains(&((b[1] + b[3]) / 2.0))
                    && (b[0] - r[2]).max(r[0] - b[2]) * width
                        < (b[3] - b[1]).max(r[3] - r[1]) * height =>
            {
                *r = bounds(&[*r, b]);
                row.push(line);
            }
            _ => rows.push((b, vec![line])),
        }
    }
    for (_, row) in &mut rows {
        row.sort_by(|a, b| a.1[0][0].total_cmp(&b.1[0][0]));
    }
    rows.into_iter().flat_map(|(_, row)| row).collect()
}

/// The engine for the user's profile languages, or else the first installed
/// OCR language.
fn engine() -> Result<OcrEngine, String> {
    let installed = OcrEngine::AvailableRecognizerLanguages()
        .and_then(|languages| languages.First()?.next().ok_or_else(windows::core::Error::empty))
        .map_err(|_| {
            "No text recognition language is installed. Add a language with text recognition in Settings > Time & language > Language, then try again.".to_string()
        })?;
    OcrEngine::TryCreateFromUserProfileLanguages()
        .or_else(|_| OcrEngine::TryCreateFromLanguage(&installed))
        .map_err(|e| format!("Windows could not start text recognition. {e}"))
}

/// One line as text and per-char boxes. The text matches the line's own text,
/// so word separators (a space, or none in Chinese and Japanese) stay as Windows gives them.
fn line_layer(
    line: &OcrLine,
    width: f32,
    height: f32,
) -> windows::core::Result<(String, Vec<NormRect>)> {
    let full = line.Text()?.to_string();
    let mut text = String::new();
    let mut boxes: Vec<NormRect> = Vec::new();
    let mut rest = full.as_str();
    for word in line.Words()? {
        let word_text = word.Text()?.to_string();
        let rect = word.BoundingRect()?;
        let (left, top) = (rect.X / width, rect.Y / height);
        let (right, bottom) = (
            (rect.X + rect.Width) / width,
            (rect.Y + rect.Height) / height,
        );
        let gap = match rest.find(&word_text) {
            Some(at) => &rest[..at],
            None if text.is_empty() => "",
            None => " ",
        };
        for c in gap.chars() {
            let at = boxes.last().map_or(left, |b| b[2]);
            text.push(c);
            boxes.push([at, top, at, bottom]);
        }
        let count = word_text.chars().count().max(1) as f32;
        for (i, c) in word_text.chars().enumerate() {
            let step = (right - left) / count;
            text.push(c);
            boxes.push([
                left + step * i as f32,
                top,
                left + step * (i + 1) as f32,
                bottom,
            ]);
        }
        rest = rest
            .find(&word_text)
            .map_or("", |at| &rest[at + word_text.len()..]);
    }
    Ok((text, boxes))
}
