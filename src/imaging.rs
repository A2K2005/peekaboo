use crate::model::{fit_size, frame_bytes, AnnotationKind, Frame, ImageEdit};
use std::{
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use windows::{
    core::{w, Interface, PCWSTR, PWSTR},
    Win32::{
        Foundation::{GENERIC_READ, GENERIC_WRITE},
        Graphics::Imaging::*,
        Storage::FileSystem::{MoveFileExW, MOVE_FILE_FLAGS},
        System::{
            Com::{
                CoCreateInstance,
                StructuredStorage::{PropVariantClear, PropVariantToUInt16, PROPBAG2, PROPVARIANT},
                CLSCTX_INPROC_SERVER,
            },
            Variant::{VARIANT, VT_R4},
        },
    },
};
fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
fn err(e: windows::core::Error) -> String {
    e.to_string()
}
unsafe fn factory() -> Result<IWICImagingFactory, String> {
    CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(err)
}

pub fn decode(path: &Path, max_width: u32, max_height: u32) -> Result<Frame, String> {
    decode_edited(path, max_width, max_height, &[])
}
pub fn decode_edited(
    path: &Path,
    max_width: u32,
    max_height: u32,
    edits: &[ImageEdit],
) -> Result<Frame, String> {
    unsafe {
        let factory = factory()?;
        let source = edited_source(&factory, path, edits).map_err(|e| {
            format!("Could not open this image. It may be damaged or need a Windows codec. {e}")
        })?;
        let (source_width, source_height) = size(&source)?;
        let (width, height) = fit_size(source_width, source_height, max_width, max_height)?;
        let scaler = factory.CreateBitmapScaler().map_err(err)?;
        scaler
            .Initialize(&source, width, height, WICBitmapInterpolationModeFant)
            .map_err(err)?;
        let converter = factory.CreateFormatConverter().map_err(err)?;
        converter
            .Initialize(
                &scaler,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
            .map_err(err)?;
        let mut pixels = vec![0; frame_bytes(width, height)?];
        converter
            .CopyPixels(std::ptr::null(), width * 4, &mut pixels)
            .map_err(err)?;
        Ok(Frame {
            width,
            height,
            pixels,
            page_count: 1,
            source_width,
            source_height,
        })
    }
}
unsafe fn size(source: &IWICBitmapSource) -> Result<(u32, u32), String> {
    let (mut width, mut height) = (0, 0);
    source.GetSize(&mut width, &mut height).map_err(err)?;
    validate_dimensions(width, height)?;
    Ok((width, height))
}
fn validate_dimensions(width: u32, height: u32) -> Result<(), String> {
    if width == 0
        || height == 0
        || width > i32::MAX as u32
        || height > i32::MAX as u32
        || width as u64 * height as u64 > 200_000_000
    {
        Err("Image dimensions must be positive and at most 200 megapixels.".into())
    } else {
        Ok(())
    }
}
unsafe fn rotated(
    factory: &IWICImagingFactory,
    source: &IWICBitmapSource,
    transform: WICBitmapTransformOptions,
) -> Result<IWICBitmapSource, String> {
    // A rotator reads its source column by column. On a streaming decoder that
    // decodes again and again: a 24 MP JPEG rotated export ran over 10 minutes.
    let cached: IWICBitmapSource = match source.cast::<IWICBitmap>() {
        Ok(bitmap) => bitmap.cast().map_err(err)?,
        Err(_) => factory
            .CreateBitmapFromSource(source, WICBitmapCacheOnLoad)
            .map_err(err)?
            .cast()
            .map_err(err)?,
    };
    let rotator = factory.CreateBitmapFlipRotator().map_err(err)?;
    rotator.Initialize(&cached, transform).map_err(err)?;
    rotator.cast().map_err(err)
}
unsafe fn edited_source(
    factory: &IWICImagingFactory,
    path: &Path,
    edits: &[ImageEdit],
) -> Result<IWICBitmapSource, String> {
    let filename = wide(path);
    let frame = factory
        .CreateDecoderFromFilename(
            PCWSTR(filename.as_ptr()),
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )
        .and_then(|decoder| decoder.GetFrame(0));
    let (mut source, orientation) = match frame {
        Ok(frame) => (
            frame.cast::<IWICBitmapSource>().map_err(err)?,
            read_orientation(&frame),
        ),
        Err(_)
            if path
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("webp")) =>
        {
            (webp_source(factory, path)?, 1)
        }
        Err(error) => return Err(error.to_string()),
    };
    size(&source)?;
    let transform = match orientation {
        2 => WICBitmapTransformFlipHorizontal,
        3 => WICBitmapTransformRotate180,
        4 => WICBitmapTransformFlipVertical,
        5 => WICBitmapTransformOptions(
            WICBitmapTransformRotate90.0 | WICBitmapTransformFlipHorizontal.0,
        ),
        6 => WICBitmapTransformRotate90,
        7 => WICBitmapTransformOptions(
            WICBitmapTransformRotate270.0 | WICBitmapTransformFlipHorizontal.0,
        ),
        8 => WICBitmapTransformRotate270,
        _ => WICBitmapTransformRotate0,
    };
    if orientation != 1 {
        source = rotated(factory, &source, transform)?;
    }
    for edit in edits {
        source = match edit.clone() {
            ImageEdit::RotateRight => rotated(factory, &source, WICBitmapTransformRotate90)?,
            ImageEdit::FlipHorizontal => {
                rotated(factory, &source, WICBitmapTransformFlipHorizontal)?
            }
            ImageEdit::Crop {
                left,
                top,
                right,
                bottom,
            } => {
                let (width, height) = size(&source)?;
                let rect = crop_rect(width, height, left, top, right, bottom)?;
                let clipper = factory.CreateBitmapClipper().map_err(err)?;
                clipper.Initialize(&source, &rect).map_err(err)?;
                clipper.cast().map_err(err)?
            }
            ImageEdit::Resize { width, height } => {
                validate_dimensions(width, height)?;
                let scaler = factory.CreateBitmapScaler().map_err(err)?;
                scaler
                    .Initialize(&source, width, height, WICBitmapInterpolationModeFant)
                    .map_err(err)?;
                scaler.cast().map_err(err)?
            }
            ImageEdit::Annotate { kind, points, text } => {
                annotated(factory, &source, kind, &points, &text)?
            }
        };
    }
    Ok(source)
}

unsafe fn webp_source(
    factory: &IWICImagingFactory,
    path: &Path,
) -> Result<IWICBitmapSource, String> {
    let file = std::io::BufReader::new(std::fs::File::open(path).map_err(|e| e.to_string())?);
    let mut decoder = image_webp::WebPDecoder::new(file).map_err(|e| e.to_string())?;
    let (width, height) = decoder.dimensions();
    validate_dimensions(width, height)?;
    let length = decoder
        .output_buffer_size()
        .filter(|n| *n <= 256 * 1024 * 1024)
        .ok_or("This WebP needs too much memory. Resize it before opening.")?;
    let channels = if decoder.has_alpha() { 4 } else { 3 };
    let format = if channels == 4 {
        GUID_WICPixelFormat32bppRGBA
    } else {
        GUID_WICPixelFormat24bppRGB
    };
    let mut pixels = vec![0u8; length];
    decoder.read_image(&mut pixels).map_err(|e| e.to_string())?;
    factory
        .CreateBitmapFromMemory(width, height, &format, width * channels, &pixels)
        .map_err(err)?
        .cast()
        .map_err(err)
}

unsafe fn annotated(
    factory: &IWICImagingFactory,
    source: &IWICBitmapSource,
    kind: AnnotationKind,
    points: &[[f32; 2]],
    text: &str,
) -> Result<IWICBitmapSource, String> {
    use windows::Win32::Graphics::{
        Direct2D::Common::*, Direct2D::*, DirectWrite::*, Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
    };
    let (width, height) = size(source)?;
    if width as u64 * height as u64 > 50_000_000 {
        return Err("Resize images above 50 megapixels before marking them up.".into());
    }
    if points.is_empty()
        || points.len() > 50_000
        || points
            .iter()
            .flatten()
            .any(|n| !n.is_finite() || !(0.0..=1.0).contains(n))
    {
        return Err("Draw inside the image.".into());
    }
    if !matches!(kind, AnnotationKind::Text | AnnotationKind::Note) && points.len() < 2 {
        return Err("Drag to draw this annotation.".into());
    }
    if text.encode_utf16().count() > 4096 {
        return Err("Use at most 4096 characters in one text annotation.".into());
    }
    let converter = factory.CreateFormatConverter().map_err(err)?;
    converter
        .Initialize(
            source,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )
        .map_err(err)?;
    let bitmap = factory
        .CreateBitmapFromSource(&converter, WICBitmapCacheOnLoad)
        .map_err(err)?;
    let d2d: ID2D1Factory =
        D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).map_err(err)?;
    let target = d2d
        .CreateWicBitmapRenderTarget(
            &bitmap,
            &D2D1_RENDER_TARGET_PROPERTIES {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                ..Default::default()
            },
        )
        .map_err(err)?;
    let color = if kind == AnnotationKind::Highlight {
        D2D1_COLOR_F {
            r: 1.0,
            g: 0.85,
            b: 0.0,
            a: 0.35,
        }
    } else {
        D2D1_COLOR_F {
            r: 0.06,
            g: 0.25,
            b: 0.78,
            a: 1.0,
        }
    };
    let brush = target.CreateSolidColorBrush(&color, None).map_err(err)?;
    let to_point = |p: [f32; 2]| windows_numerics::Vector2 {
        X: p[0] * width as f32,
        Y: p[1] * height as f32,
    };
    let a = to_point(points[0]);
    let b = to_point(*points.last().ok_or("Missing annotation point")?);
    let rect = D2D_RECT_F {
        left: a.X.min(b.X),
        top: a.Y.min(b.Y),
        right: a.X.max(b.X),
        bottom: a.Y.max(b.Y),
    };
    let thickness = (width.max(height) as f32 * 0.003).max(1.0);
    target.BeginDraw();
    match kind {
        AnnotationKind::Ink => {
            for pair in points.windows(2) {
                target.DrawLine(
                    to_point(pair[0]),
                    to_point(pair[1]),
                    &brush,
                    thickness,
                    None,
                );
            }
        }
        AnnotationKind::Highlight => target.FillRectangle(&rect, &brush),
        AnnotationKind::Rectangle => target.DrawRectangle(&rect, &brush, thickness, None),
        AnnotationKind::Ellipse => target.DrawEllipse(
            &D2D1_ELLIPSE {
                point: windows_numerics::Vector2 {
                    X: (rect.left + rect.right) / 2.0,
                    Y: (rect.top + rect.bottom) / 2.0,
                },
                radiusX: (rect.right - rect.left) / 2.0,
                radiusY: (rect.bottom - rect.top) / 2.0,
            },
            &brush,
            thickness,
            None,
        ),
        AnnotationKind::Underline | AnnotationKind::Strikeout => {
            let y = if kind == AnnotationKind::Underline {
                rect.bottom
            } else {
                (rect.top + rect.bottom) / 2.0
            };
            target.DrawLine(
                windows_numerics::Vector2 { X: rect.left, Y: y },
                windows_numerics::Vector2 {
                    X: rect.right,
                    Y: y,
                },
                &brush,
                thickness,
                None,
            );
        }
        AnnotationKind::Arrow => {
            target.DrawLine(a, b, &brush, thickness, None);
            let angle = (b.Y - a.Y).atan2(b.X - a.X);
            let length = (width.max(height) as f32 * 0.025).max(8.0);
            for offset in [-0.5f32, 0.5] {
                let tip = windows_numerics::Vector2 {
                    X: b.X - length * (angle + offset).cos(),
                    Y: b.Y - length * (angle + offset).sin(),
                };
                target.DrawLine(b, tip, &brush, thickness, None);
            }
        }
        AnnotationKind::Text | AnnotationKind::Note => {
            let dwrite: IDWriteFactory =
                DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).map_err(err)?;
            let format = dwrite
                .CreateTextFormat(
                    w!("Segoe UI"),
                    None,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    (width as f32 * 0.03).max(12.0),
                    w!("en-us"),
                )
                .map_err(err)?;
            let bounds = D2D_RECT_F {
                left: a.X,
                top: a.Y,
                right: (a.X + width as f32 * 0.6).min(width as f32),
                bottom: height as f32,
            };
            let text: Vec<u16> = text.encode_utf16().collect();
            target.DrawText(
                &text,
                &format,
                &bounds,
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }
    target.EndDraw(None, None).map_err(err)?;
    bitmap.cast().map_err(err)
}
fn crop_rect(
    width: u32,
    height: u32,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
) -> Result<WICRect, String> {
    if [left, top, right, bottom]
        .iter()
        .any(|n| !n.is_finite() || !(0.0..=1.0).contains(n))
        || right <= left
        || bottom <= top
    {
        return Err("Select a crop rectangle inside the image.".into());
    }
    let x = (left as f64 * width as f64).round() as i32;
    let y = (top as f64 * height as f64).round() as i32;
    let r = (right as f64 * width as f64).round().min(width as f64) as i32;
    let b = (bottom as f64 * height as f64).round().min(height as f64) as i32;
    if r <= x || b <= y {
        return Err("The crop selection is too small.".into());
    }
    Ok(WICRect {
        X: x,
        Y: y,
        Width: r - x,
        Height: b - y,
    })
}
/// Rebuild from the original. Never export a display-resolution preview.
pub fn export(path: &Path, output: &Path, edits: &[ImageEdit]) -> Result<(), String> {
    unsafe {
        let factory = factory()?;
        let source = edited_source(&factory, path, edits)?;
        write_source(&factory, &source, output)
    }
}
pub fn export_frame(frame: &Frame, output: &Path) -> Result<(), String> {
    if frame_bytes(frame.width, frame.height)? != frame.pixels.len() {
        return Err("Invalid image buffer.".into());
    }
    unsafe {
        let factory = factory()?;
        let bitmap = factory
            .CreateBitmapFromMemory(
                frame.width,
                frame.height,
                &GUID_WICPixelFormat32bppPBGRA,
                frame.width * 4,
                &frame.pixels,
            )
            .map_err(err)?;
        write_source(&factory, &bitmap.cast().map_err(err)?, output)
    }
}
struct TempFile(PathBuf);
impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
unsafe fn write_source(
    factory: &IWICImagingFactory,
    source: &IWICBitmapSource,
    output: &Path,
) -> Result<(), String> {
    if output.exists() {
        return Err("A file already exists there. Choose a new name to keep both files.".into());
    }
    let extension = output
        .extension()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if extension == "webp" {
        return write_webp(factory, source, output);
    }
    let (container, jpeg) = match extension.as_str() {
        "png" => (GUID_ContainerFormatPng, false),
        "jpg" | "jpeg" => (GUID_ContainerFormatJpeg, true),
        "tif" | "tiff" => (GUID_ContainerFormatTiff, false),
        "bmp" => (GUID_ContainerFormatBmp, false),
        _ => return Err("Choose PNG, JPEG, WebP, TIFF, or BMP for image export.".into()),
    };
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temp_path = parent.join(format!(
        ".preview-{}-{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let reserved = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .map_err(|e| e.to_string())?;
    drop(reserved);
    let temp = TempFile(temp_path);
    {
        let filename = wide(&temp.0);
        let stream = factory.CreateStream().map_err(err)?;
        stream
            .InitializeFromFilename(PCWSTR(filename.as_ptr()), GENERIC_WRITE.0)
            .map_err(err)?;
        let encoder = factory
            .CreateEncoder(&container, std::ptr::null())
            .map_err(err)?;
        encoder
            .Initialize(&stream, WICBitmapEncoderNoCache)
            .map_err(err)?;
        let (mut frame, mut options) = (None, None);
        encoder
            .CreateNewFrame(&mut frame, &mut options)
            .map_err(err)?;
        if jpeg {
            if let Some(options) = &options {
                let mut name: Vec<u16> = "ImageQuality".encode_utf16().chain(Some(0)).collect();
                let property = PROPBAG2 {
                    pstrName: PWSTR(name.as_mut_ptr()),
                    ..Default::default()
                };
                let mut quality = VARIANT::default();
                (*quality.Anonymous.Anonymous).vt = VT_R4;
                (*quality.Anonymous.Anonymous).Anonymous.fltVal = 0.92;
                options.Write(1, &property, &quality).map_err(err)?;
            }
        }
        let frame = frame.ok_or("Windows did not create an output encoder.")?;
        frame.Initialize(options.as_ref()).map_err(err)?;
        let (width, height) = size(source)?;
        frame.SetSize(width, height).map_err(err)?;
        let mut pixel_format = if jpeg {
            GUID_WICPixelFormat24bppBGR
        } else {
            GUID_WICPixelFormat32bppBGRA
        };
        frame.SetPixelFormat(&mut pixel_format).map_err(err)?;
        let converter = factory.CreateFormatConverter().map_err(err)?;
        if jpeg {
            frame_bytes(width, 1)?;
            converter
                .Initialize(
                    source,
                    &GUID_WICPixelFormat32bppPBGRA,
                    WICBitmapDitherTypeNone,
                    None,
                    0.0,
                    WICBitmapPaletteTypeCustom,
                )
                .map_err(err)?;
            let mut rgba = vec![0u8; width as usize * 4];
            let mut rgb = vec![0u8; width as usize * 3];
            for row in 0..height {
                let rect = WICRect {
                    X: 0,
                    Y: row as i32,
                    Width: width as i32,
                    Height: 1,
                };
                converter
                    .CopyPixels(&rect, width * 4, &mut rgba)
                    .map_err(err)?;
                for (input, output) in rgba.chunks_exact(4).zip(rgb.chunks_exact_mut(3)) {
                    let white = 255 - input[3];
                    for channel in 0..3 {
                        output[channel] = input[channel].saturating_add(white);
                    }
                }
                frame.WritePixels(1, width * 3, &rgb).map_err(err)?;
            }
        } else {
            converter
                .Initialize(
                    source,
                    &pixel_format,
                    WICBitmapDitherTypeNone,
                    None,
                    0.0,
                    WICBitmapPaletteTypeCustom,
                )
                .map_err(err)?;
            frame
                .WriteSource(&converter, std::ptr::null())
                .map_err(err)?;
        }
        frame.Commit().map_err(err)?;
        encoder.Commit().map_err(err)?;
    }
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&temp.0)
        .and_then(|file| file.sync_all())
        .map_err(|e| e.to_string())?;
    let check_path = wide(&temp.0);
    let check = factory
        .CreateDecoderFromFilename(
            PCWSTR(check_path.as_ptr()),
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )
        .map_err(err)?;
    let check_frame: IWICBitmapSource = check.GetFrame(0).map_err(err)?.cast().map_err(err)?;
    if size(&check_frame)? != size(source)? {
        return Err("The exported image dimensions did not match. No output was saved.".into());
    }
    drop(check_frame);
    drop(check);
    let from = wide(&temp.0);
    let to = wide(output);
    MoveFileExW(
        PCWSTR(from.as_ptr()),
        PCWSTR(to.as_ptr()),
        MOVE_FILE_FLAGS(0),
    )
    .map_err(err)?;
    Ok(())
}

unsafe fn write_webp(
    factory: &IWICImagingFactory,
    source: &IWICBitmapSource,
    output: &Path,
) -> Result<(), String> {
    let (width, height) = size(source)?;
    let length = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .filter(|n| *n <= 256 * 1024 * 1024)
        .ok_or("Resize images above 64 megapixels before exporting WebP.")?;
    let converter = factory.CreateFormatConverter().map_err(err)?;
    converter
        .Initialize(
            source,
            &GUID_WICPixelFormat32bppRGBA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )
        .map_err(err)?;
    let mut pixels = vec![0u8; length];
    converter
        .CopyPixels(std::ptr::null(), width * 4, &mut pixels)
        .map_err(err)?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let path = parent.join(format!(
        ".preview-{}-{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    let temp = TempFile(path);
    image_webp::WebPEncoder::new(&mut file)
        .encode(&pixels, width, height, image_webp::ColorType::Rgba8)
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    let check = image_webp::WebPDecoder::new(std::io::BufReader::new(
        std::fs::File::open(&temp.0).map_err(|e| e.to_string())?,
    ))
    .map_err(|e| e.to_string())?;
    if check.dimensions() != (width, height) {
        return Err("WebP verification failed. No output was saved.".into());
    }
    drop(check);
    let from = wide(&temp.0);
    let to = wide(output);
    MoveFileExW(
        PCWSTR(from.as_ptr()),
        PCWSTR(to.as_ptr()),
        MOVE_FILE_FLAGS(0),
    )
    .map_err(err)?;
    Ok(())
}
unsafe fn read_orientation(frame: &IWICBitmapFrameDecode) -> u16 {
    let Ok(reader) = frame.GetMetadataQueryReader() else {
        return 1;
    };
    for query in [w!("/app1/ifd/{ushort=274}"), w!("/ifd/{ushort=274}")] {
        let mut value = PROPVARIANT::default();
        let result = reader.GetMetadataByName(query, &mut value);
        let orientation = if result.is_ok() {
            PropVariantToUInt16(&value).ok()
        } else {
            None
        };
        let _ = PropVariantClear(&mut value);
        if let Some(n @ 1..=8) = orientation {
            return n;
        }
    }
    1
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crop_bounds() {
        let r = crop_rect(100, 200, 0.1, 0.2, 0.8, 0.9).unwrap();
        assert_eq!((r.X, r.Y, r.Width, r.Height), (10, 40, 70, 140));
        assert!(crop_rect(100, 100, f32::NAN, 0.0, 1.0, 1.0).is_err());
        assert!(crop_rect(100, 100, 0.8, 0.0, 0.2, 1.0).is_err());
        assert!(crop_rect(100, 100, -0.1, 0.0, 1.0, 1.0).is_err());
    }
}
