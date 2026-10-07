//! Background removal with the withoutbg Snap models (Apache-2.0):
//! https://huggingface.co/withoutbg/snap. Three stages: Depth Anything V2
//! Small estimates depth, a matting model finds a coarse alpha at 256 x 256,
//! and a refiner sharpens it at up to 1024 pixels. The alpha is then scaled
//! to the full image. ONNX Runtime and the models load on first use.
use crate::model::{fit_size, Frame, ImageEdit};
use ort::{session::Session, value::Tensor};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Instant,
};

const DEPTH: &str = "depth_anything_v2_vits_slim.onnx";
const MATTING: &str = "snap_matting_0.1.0.onnx";
const REFINER: &str = "snap_refiner_0.1.0.onnx";
/// The refiner's longest side. Full resolution took 4.6 s for 12 MP.
const REFINE: u32 = 1024;
/// The depth input's shorter side. 266 kept quality close to the 518 the
/// authors use (subject IoU 0.900 vs 0.910) at a quarter of the cost.
const DEPTH_SIDE: u32 = 266;

struct Snap {
    depth: Session,
    matting: Session,
    refiner: Session,
}
thread_local! {static MODEL: RefCell<Option<Snap>> = const { RefCell::new(None) };}
static RUNTIME: OnceLock<Result<(), String>> = OnceLock::new();

/// The AI pack folder: beside the app, in %LOCALAPPDATA%, or runtime/ai in development.
fn pack() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("Cannot locate the app.")?;
    let mut paths = vec![dir.join("ai")];
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        paths.push(PathBuf::from(local).join("PreviewForWindows").join("ai"));
    }
    #[cfg(test)]
    paths.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runtime/ai"));
    if let Some(root) = dir.parent().and_then(Path::parent) {
        paths.push(root.join("runtime/ai"));
    }
    paths
        .into_iter()
        .find(|p| [ "onnxruntime.dll", DEPTH, MATTING, REFINER].iter().all(|f| p.join(f).is_file()))
        .ok_or_else(|| "Background removal needs the AI pack, which is not installed. Add the AI pack beside Preview, then try again. No image was uploaded.".into())
}
fn load(pack: &Path) -> Result<Snap, String> {
    RUNTIME
        .get_or_init(|| {
            ort::init_from(pack.join("onnxruntime.dll"))
                .map(|environment| {
                    environment.commit();
                })
                .map_err(|e| format!("Cannot load background removal: {e}"))
        })
        .clone()?;
    // CPU only. DirectML hung the GPU with the earlier BiRefNet model (see report).
    let open = |name: &str| {
        Session::builder()
            .map_err(|e| e.to_string())?
            .commit_from_file(pack.join(name))
            .map_err(|e| format!("Cannot load the background model {name}: {e}"))
    };
    Ok(Snap {
        depth: open(DEPTH)?,
        matting: open(MATTING)?,
        refiner: open(REFINER)?,
    })
}
/// Run one model and return its first output's height, width, and values.
fn run(
    session: &mut Session,
    input: &str,
    output: &str,
    shape: [usize; 4],
    data: Vec<f32>,
) -> Result<(usize, usize, Vec<f32>), String> {
    let tensor = Tensor::from_array((shape, data)).map_err(|e| e.to_string())?;
    let outputs = session
        .run(ort::inputs![input => tensor])
        .map_err(|e| format!("Background removal failed: {e}"))?;
    let value = outputs
        .get(output)
        .ok_or("The AI pack has the wrong background model.")?;
    let (shape, values) = value
        .try_extract_tensor::<f32>()
        .map_err(|e| e.to_string())?;
    let size = |i: usize| shape.len().checked_sub(i).map(|i| shape[i] as usize);
    let (height, width) = (size(2).unwrap_or(0), size(1).unwrap_or(0));
    if height * width != values.len() || values.iter().any(|v| !v.is_finite()) {
        return Err("The background model returned an invalid mask.".into());
    }
    Ok((height, width, values.to_vec()))
}

/// Remove the background. Returns the full-resolution cut-out: premultiplied
/// BGRA with a transparent background. Call on a COM-initialized worker.
pub fn cutout(path: &Path, edits: &[ImageEdit]) -> Result<Frame, String> {
    let mut original = crate::imaging::decode_edited(path, u32::MAX, u32::MAX, edits)
        .map_err(|e| format!("Cannot prepare background removal. Images above 16 megapixels must be resized first. {e}"))?;
    let (width, height) = fit_size(original.width, original.height, REFINE, REFINE)?;
    let refine = crate::imaging::resize_frame(&original, width, height)?;
    let (depth_width, depth_height) = depth_size(width, height);
    let depth_input = crate::imaging::resize_frame(&refine, depth_width, depth_height)?;
    let small = crate::imaging::resize_frame(&refine, 256, 256)?;
    let alpha = MODEL.with(|slot| -> Result<Vec<f32>, String> {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Some(load(&pack()?)?);
        }
        let snap = slot.as_mut().ok_or("Cannot start background removal.")?;
        let (dw, dh) = (depth_width as usize, depth_height as usize);
        let (h, w, mut depth) = run(
            &mut snap.depth,
            "image",
            "depth",
            [1, 3, dh, dw],
            planes(&depth_input, true),
        )?;
        // Inverse depth scaled to 0..1, as the Snap pipeline expects.
        let (low, high) = depth
            .iter()
            .fold((f32::MAX, f32::MIN), |(l, h), &v| (l.min(v), h.max(v)));
        depth
            .iter_mut()
            .for_each(|v| *v = (*v - low) / (high - low).max(1e-6));
        let mut input = planes(&small, false);
        input.extend(resize_plane(&depth, w, h, 256, 256));
        let (_, _, coarse) = run(
            &mut snap.matting,
            "rgbd_input",
            "alpha_output",
            [1, 4, 256, 256],
            input,
        )?;
        let coarse: Vec<f32> = coarse.iter().map(|v| v.clamp(0.0, 1.0)).collect();
        let (rw, rh) = (width as usize, height as usize);
        let mut input = planes(&refine, false);
        input.extend(resize_plane(&depth, w, h, rw, rh));
        input.extend(resize_plane(&coarse, 256, 256, rw, rh));
        let (_, _, fine) = run(
            &mut snap.refiner,
            "rgbd_alpha_input",
            "alpha_output",
            [1, 5, rh, rw],
            input,
        )?;
        Ok(fine.iter().map(|v| v.clamp(0.0, 1.0)).collect())
    })?;
    apply_alpha(&mut original, &alpha, width as usize, height as usize);
    Ok(original)
}
/// Multiply every pixel by the alpha mask scaled up bilinearly. Rows run on
/// several threads: one thread took 0.65 s for 12 MP.
fn apply_alpha(frame: &mut Frame, alpha: &[f32], width: usize, height: usize) {
    let (fw, fh) = (frame.width as usize, frame.height as usize);
    let axis = |i: usize, from: usize, to: usize| {
        let s = ((i as f32 + 0.5) * from as f32 / to as f32 - 0.5).clamp(0.0, (from - 1) as f32);
        let low = s as usize;
        (low, (low + 1).min(from - 1), s - low as f32)
    };
    let columns: Vec<_> = (0..fw).map(|x| axis(x, width, fw)).collect();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let rows = fh.div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        for (part, chunk) in frame.pixels.chunks_mut(rows * fw * 4).enumerate() {
            let columns = &columns;
            scope.spawn(move || {
                for (i, row) in chunk.chunks_exact_mut(fw * 4).enumerate() {
                    let (y0, y1, fy) = axis(part * rows + i, height, fh);
                    let (top, bottom) = (&alpha[y0 * width..], &alpha[y1 * width..]);
                    for (pixel, &(x0, x1, fx)) in row.chunks_exact_mut(4).zip(columns) {
                        let upper = top[x0] + (top[x1] - top[x0]) * fx;
                        let lower = bottom[x0] + (bottom[x1] - bottom[x0]) * fx;
                        let a = ((upper + (lower - upper) * fy) * 255.0 + 0.5) as u32;
                        for c in pixel.iter_mut() {
                            *c = ((*c as u32 * a + 127) / 255) as u8;
                        }
                    }
                }
            });
        }
    });
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
    let result = cutout(path, edits)?;
    crate::imaging::export_frame(&result, output)?;
    Ok(format!(
        "Background removed locally in {:.1} seconds. Saved {} × {} PNG.",
        started.elapsed().as_secs_f32(),
        result.width,
        result.height
    ))
}
/// Depth input size: the shorter side near DEPTH_SIDE, both sides multiples
/// of 14 (the ViT patch size) and at most 1022.
fn depth_size(width: u32, height: u32) -> (u32, u32) {
    let scale = DEPTH_SIDE as f32 / width.min(height) as f32;
    let side = |n: u32| ((n as f32 * scale / 14.0).round() as u32 * 14).clamp(DEPTH_SIDE, 1022);
    (side(width), side(height))
}
/// RGB planes in 0..1, or ImageNet-normalized, from premultiplied BGRA
/// composited onto white.
fn planes(frame: &Frame, imagenet: bool) -> Vec<f32> {
    let (mean, deviation) = if imagenet {
        ([0.485f32, 0.456, 0.406], [0.229f32, 0.224, 0.225])
    } else {
        ([0.0; 3], [1.0; 3])
    };
    let plane = frame.width as usize * frame.height as usize;
    let mut data = vec![0.0f32; plane * 3];
    for (i, pixel) in frame.pixels.chunks_exact(4).enumerate() {
        let white = 255 - pixel[3];
        for c in 0..3 {
            let value = pixel[2 - c].saturating_add(white) as f32 / 255.0;
            data[c * plane + i] = (value - mean[c]) / deviation[c];
        }
    }
    data
}
/// Bilinear resize of one plane, pixel centers aligned.
fn resize_plane(src: &[f32], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(dw * dh);
    for y in 0..dh {
        let sy = (y as f32 + 0.5) * sh as f32 / dh as f32 - 0.5;
        for x in 0..dw {
            let sx = (x as f32 + 0.5) * sw as f32 / dw as f32 - 0.5;
            out.push(sample(src, sw, sh, sx, sy));
        }
    }
    out
}
fn sample(mask: &[f32], width: usize, height: usize, x: f32, y: f32) -> f32 {
    let x = x.clamp(0.0, (width - 1) as f32);
    let y = y.clamp(0.0, (height - 1) as f32);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(width - 1), (y0 + 1).min(height - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let top = mask[y0 * width + x0] * (1.0 - fx) + mask[y0 * width + x1] * fx;
    let bottom = mask[y1 * width + x0] * (1.0 - fx) + mask[y1 * width + x1] * fx;
    top * (1.0 - fy) + bottom * fy
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn depth_sizes_are_patch_multiples() {
        assert_eq!(depth_size(1024, 768), (350, 266));
        assert_eq!(depth_size(768, 1024), (266, 350));
        assert_eq!(depth_size(1024, 100), (1022, 266));
    }
    #[test]
    fn bilinear_plane_resize() {
        // 2 x 1 ramp to 4 x 1: centers at 0.25, 0.75 of the source span.
        assert_eq!(
            resize_plane(&[0.0, 1.0], 2, 1, 4, 1),
            vec![0.0, 0.25, 0.75, 1.0]
        );
        assert_eq!(resize_plane(&[0.0, 1.0, 1.0, 0.0], 2, 2, 1, 1), vec![0.5]);
        let planes = planes(
            &Frame {
                width: 1,
                height: 1,
                pixels: vec![0, 0, 0, 0],
                page_count: 1,
                source_width: 1,
                source_height: 1,
            },
            false,
        );
        assert_eq!(
            planes,
            vec![1.0, 1.0, 1.0],
            "Transparent pixels must read as white"
        );
    }
}
