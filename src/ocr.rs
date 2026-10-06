use crate::model::ImageEdit;
use std::path::Path;
use windows::{
    Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap},
    Media::Ocr::OcrEngine,
    Storage::Streams::DataWriter,
};

/// Call on the COM-initialized worker, never on the window thread.
pub fn recognize(path: &Path, edits: &[ImageEdit]) -> Result<String, String> {
    let maximum = OcrEngine::MaxImageDimension()
        .map_err(|e| format!("Windows text recognition is unavailable on this PC: {e}"))?
        .min(4096);
    if maximum == 0 {
        return Err("Windows text recognition reported an invalid image limit.".into());
    }
    let engine = OcrEngine::TryCreateFromUserProfileLanguages().map_err(|e|
        format!("Windows could not start text recognition for your preferred languages. Install a supported language's OCR capability in Windows Settings, then try again. No image was uploaded. {e}"))?;
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
    drop(frame);
    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(|e| format!("Windows could not start reading this image: {e}"))?
        .join()
        .map_err(|e| format!("Windows could not read text from this image: {e}"))?;
    let text = result
        .Text()
        .map_err(|e| format!("Windows could not return the recognized text: {e}"))?
        .to_string();
    if text.trim().is_empty() {
        return Err(
            "No text was recognized. Try a clearer image or check the installed OCR language."
                .into(),
        );
    }
    Ok(text)
}
