use crate::model::ImageEdit;
use ort::{session::Session, value::Tensor};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    time::Instant,
};
thread_local! {static SESSION:RefCell<Option<Session>>=const{RefCell::new(None)};}
fn pack() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("Cannot locate the app.")?;
    let mut paths = vec![dir.join("ai")];
    #[cfg(test)]
    paths.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runtime/ai"));
    if let Some(root) = dir.parent().and_then(Path::parent) {
        paths.push(root.join("runtime/ai"));
    }
    paths.into_iter().find(|p|p.join("onnxruntime.dll").is_file()&&p.join("birefnet_lite.onnx").is_file())
        .ok_or("The optional background-removal pack is not installed. Add the verified AI pack beside Preview, then try again. No image was uploaded.".into())
}
fn session() -> Result<Session, String> {
    let pack = pack()?;
    ort::init_from(pack.join("onnxruntime.dll"))
        .map_err(|e| format!("Cannot load background removal: {e}"))?
        .commit();
    // DirectML caused DXGI_ERROR_DEVICE_HUNG in this host's measured probe.
    // CPU inference is the verified default; no GPU provider is loaded at startup.
    Session::builder()
        .map_err(|e| e.to_string())?
        .with_intra_threads(4)
        .map_err(|e| e.to_string())?
        .commit_from_file(pack.join("birefnet_lite.onnx"))
        .map_err(|e| format!("Cannot load the background model: {e}"))
}
/// Save a new full-resolution PNG. The original and its edit history remain intact.
pub fn remove(path: &Path, output: &Path, edits: &[ImageEdit]) -> Result<String, String> {
    if output
        .extension()
        .is_none_or(|x| !x.eq_ignore_ascii_case("png"))
    {
        return Err("Save the transparent result as a PNG.".into());
    }
    if output.exists() {
        return Err("Choose a new filename so the existing file stays intact.".into());
    }
    let started = Instant::now();
    let mut original=crate::imaging::decode_edited(path,u32::MAX,u32::MAX,edits)
        .map_err(|e|format!("Cannot prepare background removal. Images above 16 megapixels must be resized first. {e}"))?;
    let mut resized = edits.to_vec();
    resized.push(ImageEdit::Resize {
        width: 1024,
        height: 1024,
    });
    let input = crate::imaging::decode_edited(path, 1024, 1024, &resized)?;
    let plane = 1024 * 1024;
    let mut data = vec![0.0f32; plane * 3];
    let means = [0.485f32, 0.456, 0.406];
    let deviations = [0.229f32, 0.224, 0.225];
    for (i, pixel) in input.pixels.chunks_exact(4).enumerate() {
        let white = 255 - pixel[3];
        for c in 0..3 {
            data[c * plane + i] =
                ((pixel[2 - c].saturating_add(white) as f32 / 255.0) - means[c]) / deviations[c];
        }
    }
    let mask = SESSION.with(|slot| -> Result<Vec<f32>, String> {
        let mut guard = slot.borrow_mut();
        if guard.is_none() {
            *guard = Some(session()?);
        }
        let session = guard.as_mut().ok_or("Cannot start background removal.")?;
        let tensor =
            Tensor::from_array(([1usize, 3, 1024, 1024], data)).map_err(|e| e.to_string())?;
        let output = session
            .run(ort::inputs!["input_image"=>tensor])
            .map_err(|e| format!("Background removal failed: {e}"))?;
        let output = output
            .get("output_image")
            .ok_or("The AI pack is not the supported BiRefNet model: output_image is missing.")?;
        let (shape, values) = output
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        if &shape[..] != [1, 1, 1024, 1024]
            || values.len() != plane
            || values.iter().any(|v| !v.is_finite())
        {
            return Err("The background model returned an invalid mask.".into());
        }
        // This pinned ONNX export returns logits. Apply the model's sigmoid.
        Ok(values.iter().map(|v| 1.0 / (1.0 + (-v).exp())).collect())
    })?;
    for y in 0..original.height {
        for x in 0..original.width {
            let alpha = sample(
                &mask,
                (x as f32 + 0.5) * 1024.0 / original.width as f32 - 0.5,
                (y as f32 + 0.5) * 1024.0 / original.height as f32 - 0.5,
            );
            let i = (y as usize * original.width as usize + x as usize) * 4;
            for c in 0..4 {
                original.pixels[i + c] = (original.pixels[i + c] as f32 * alpha).round() as u8;
            }
        }
    }
    crate::imaging::export_frame(&original, output)?;
    Ok(format!(
        "Background removed locally in {:.1} seconds. Saved {} × {} PNG.",
        started.elapsed().as_secs_f32(),
        original.width,
        original.height
    ))
}
fn sample(mask: &[f32], x: f32, y: f32) -> f32 {
    let x = x.clamp(0.0, 1023.0);
    let y = y.clamp(0.0, 1023.0);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(1023);
    let y1 = (y0 + 1).min(1023);
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let a = mask[y0 * 1024 + x0] * (1.0 - fx) + mask[y0 * 1024 + x1] * fx;
    let b = mask[y1 * 1024 + x0] * (1.0 - fx) + mask[y1 * 1024 + x1] * fx;
    a * (1.0 - fy) + b * fy
}
