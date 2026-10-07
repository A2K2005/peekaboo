use crate::model::{
    fit_size, frame_bytes, AnnotationKind, ExportOptions, Frame, ImageEdit, ImageFormat,
};
use std::{
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use windows::{
    core::{w, Interface, PCWSTR, PWSTR},
    Win32::{
        Foundation::{GENERIC_READ, GENERIC_WRITE, VARIANT_TRUE},
        Graphics::Imaging::*,
        Media::MediaFoundation::{
            MFMediaType_Video, MFTEnumEx, MFVideoFormat_HEVC, MFT_CATEGORY_VIDEO_DECODER,
            MFT_ENUM_FLAG_ALL, MFT_REGISTER_TYPE_INFO,
        },
        Storage::FileSystem::{MoveFileExW, MOVE_FILE_FLAGS},
        System::{
            Com::{
                CoCreateInstance, CoTaskMemFree, IStream,
                StructuredStorage::{PropVariantClear, PropVariantToUInt16, PROPBAG2, PROPVARIANT},
                CLSCTX_INPROC_SERVER, STREAM_SEEK_END,
            },
            Variant::{VARIANT, VT_BOOL, VT_R4},
        },
        UI::Shell::SHCreateMemStream,
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
        let (source, (source_width, source_height)) =
            edited_source(&factory, path, edits, Some((max_width, max_height)))?;
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
/// Returns the edited source and its full-resolution size. `display` is the
/// target box of a viewing decode. With no edits, the codec may then scale
/// natively (JPEG DCT scaling), so the source can be smaller than that size.
unsafe fn edited_source(
    factory: &IWICImagingFactory,
    path: &Path,
    edits: &[ImageEdit],
    display: Option<(u32, u32)>,
) -> Result<(IWICBitmapSource, (u32, u32)), String> {
    let filename = wide(path);
    let frame = factory
        .CreateDecoderFromFilename(
            PCWSTR(filename.as_ptr()),
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )
        .and_then(|decoder| decoder.GetFrame(0));
    let (mut source, orientation, full) = match frame {
        Ok(frame) => {
            let (width, height) = size(&frame)?;
            let orientation = read_orientation(&frame);
            let swapped = orientation >= 5;
            let full = if swapped {
                (height, width)
            } else {
                (width, height)
            };
            let reduced = match display {
                Some((max_width, max_height)) if edits.is_empty() => {
                    let (w, h) = fit_size(full.0, full.1, max_width, max_height)?;
                    reduced_frame(
                        factory,
                        &frame,
                        width,
                        if swapped { (h, w) } else { (w, h) },
                    )
                }
                _ => None,
            };
            match reduced {
                Some(source) => (source, orientation, Some(full)),
                None => (frame.cast().map_err(err)?, orientation, None),
            }
        }
        Err(_)
            if path
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("webp")) =>
        {
            (webp_source(factory, path)?, 1, None)
        }
        Err(_) if format_for(path) == Some(ImageFormat::Heic) && !heic_decode_available() => {
            return Err(format!(
                "Windows cannot open HEIC files on this PC. {HEIC_HELP}"
            ));
        }
        Err(error) => {
            return Err(format!(
                "Could not open this image. It may be damaged or need a Windows codec. {error}"
            ))
        }
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
    let full = match full {
        Some(full) => full,
        None => size(&source)?,
    };
    Ok((source, full))
}

/// Decode at the smallest native codec scale that still covers `want`,
/// in frame orientation. None when the codec cannot scale or it saves nothing.
unsafe fn reduced_frame(
    factory: &IWICImagingFactory,
    frame: &IWICBitmapFrameDecode,
    frame_width: u32,
    want: (u32, u32),
) -> Option<IWICBitmapSource> {
    let transform: IWICBitmapSourceTransform = frame.cast().ok()?;
    let (mut width, mut height) = want;
    transform.GetClosestSize(&mut width, &mut height).ok()?;
    if width >= frame_width || width < want.0 || height < want.1 {
        return None;
    }
    let mut format = GUID_WICPixelFormat32bppPBGRA;
    transform.GetClosestPixelFormat(&mut format).ok()?;
    let bitmap = factory
        .CreateBitmap(width, height, &format, WICBitmapCacheOnLoad)
        .ok()?;
    {
        let lock = bitmap
            .Lock(std::ptr::null(), WICBitmapLockWrite.0 as u32)
            .ok()?;
        let stride = lock.GetStride().ok()?;
        let (mut length, mut data) = (0, std::ptr::null_mut());
        lock.GetDataPointer(&mut length, &mut data).ok()?;
        let buffer = std::slice::from_raw_parts_mut(data, length as usize);
        transform
            .CopyPixels(
                std::ptr::null(),
                width,
                height,
                &format,
                WICBitmapTransformRotate0,
                stride,
                buffer,
            )
            .ok()?;
    }
    bitmap.cast().ok()
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
/// The output extension picks the format. JPEG quality is 0.92; WebP is lossless.
pub fn export(path: &Path, output: &Path, edits: &[ImageEdit]) -> Result<(), String> {
    export_with(path, output, edits, &default_options(output)?).map(|_| ())
}
/// Rebuild from the original with explicit options. Returns the bytes written.
pub fn export_with(
    path: &Path,
    output: &Path,
    edits: &[ImageEdit],
    options: &ExportOptions,
) -> Result<u64, String> {
    unsafe {
        let factory = factory()?;
        let (source, _) = edited_source(&factory, path, edits, None)?;
        write_source(&factory, &source, output, options)
    }
}
/// The bytes `export_with` would write. No file is written. Above a pixel
/// budget it encodes full-resolution row bands and scales the size up: a full
/// encode of a 24 MP photo took 0.3 s for JPEG, 1.3 s for PNG, 1.9 s for WebP.
#[allow(dead_code)] // The shell calls this in wave 2.
pub fn estimate_size(
    path: &Path,
    edits: &[ImageEdit],
    options: &ExportOptions,
) -> Result<u64, String> {
    unsafe {
        let factory = factory()?;
        let (source, _) = edited_source(&factory, path, edits, None)?;
        let (width, height) = size(&source)?;
        let budget = match options.format {
            ImageFormat::Jpeg | ImageFormat::Bmp => 40_000_000,
            _ => 4_000_000,
        };
        if width as u64 * height as u64 <= budget {
            return encoded_length(&factory, &source, options);
        }
        let (sample, rows) = row_sample(&factory, &source, width, height)?;
        let length = encoded_length(&factory, &sample, options)?;
        Ok((length as f64 * height as f64 / rows as f64).round() as u64)
    }
}
unsafe fn encoded_length(
    factory: &IWICImagingFactory,
    source: &IWICBitmapSource,
    options: &ExportOptions,
) -> Result<u64, String> {
    // SHCreateMemStream: a TIFF took 0.8 s here, CreateStreamOnHGlobal took 29.6 s.
    let stream = SHCreateMemStream(None).ok_or("Not enough memory to estimate the file size.")?;
    encode(factory, source, &stream, options)?;
    let mut length = 0;
    stream
        .Seek(0, STREAM_SEEK_END, Some(&mut length))
        .map_err(err)?;
    Ok(length)
}
/// Eight evenly spaced bands of full-resolution rows, about 2 MP in total,
/// stacked into one bitmap. Band heights are multiples of 16 rows to match
/// JPEG and WebP blocks. Returns the bitmap and its row count.
unsafe fn row_sample(
    factory: &IWICImagingFactory,
    source: &IWICBitmapSource,
    width: u32,
    height: u32,
) -> Result<(IWICBitmapSource, u32), String> {
    const BANDS: u32 = 8;
    let band = (2_000_000 / width / BANDS / 16 * 16)
        .max(16)
        .min(height / BANDS)
        .max(1);
    let rows = band * BANDS;
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
        .CreateBitmap(
            width,
            rows,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapCacheOnLoad,
        )
        .map_err(err)?;
    {
        let lock = bitmap
            .Lock(std::ptr::null(), WICBitmapLockWrite.0 as u32)
            .map_err(err)?;
        let stride = lock.GetStride().map_err(err)?;
        let (mut length, mut data) = (0, std::ptr::null_mut());
        lock.GetDataPointer(&mut length, &mut data).map_err(err)?;
        let buffer = std::slice::from_raw_parts_mut(data, length as usize);
        let band_bytes = band as usize * stride as usize;
        for i in 0..BANDS {
            let rect = WICRect {
                X: 0,
                Y: ((height - band) as u64 * i as u64 / (BANDS - 1) as u64) as i32,
                Width: width as i32,
                Height: band as i32,
            };
            let start = i as usize * band_bytes;
            converter
                .CopyPixels(&rect, stride, &mut buffer[start..start + band_bytes])
                .map_err(err)?;
        }
    }
    Ok((bitmap.cast().map_err(err)?, rows))
}
pub fn export_frame(frame: &Frame, output: &Path) -> Result<(), String> {
    unsafe {
        let factory = factory()?;
        let bitmap = frame_bitmap(&factory, frame)?;
        write_source(&factory, &bitmap, output, &default_options(output)?).map(|_| ())
    }
}
unsafe fn frame_bitmap(
    factory: &IWICImagingFactory,
    frame: &Frame,
) -> Result<IWICBitmapSource, String> {
    if frame_bytes(frame.width, frame.height)? != frame.pixels.len() {
        return Err("Invalid image buffer.".into());
    }
    factory
        .CreateBitmapFromMemory(
            frame.width,
            frame.height,
            &GUID_WICPixelFormat32bppPBGRA,
            frame.width * 4,
            &frame.pixels,
        )
        .map_err(err)?
        .cast()
        .map_err(err)
}
fn format_for(path: &Path) -> Option<ImageFormat> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "jpg" | "jpeg" | "jpe" | "jfif" => ImageFormat::Jpeg,
        "png" => ImageFormat::Png,
        "webp" => ImageFormat::WebP,
        "tif" | "tiff" => ImageFormat::Tiff,
        "heic" | "heif" => ImageFormat::Heic,
        "bmp" => ImageFormat::Bmp,
        _ => return None,
    })
}
fn default_options(output: &Path) -> Result<ExportOptions, String> {
    let format =
        format_for(output).ok_or("Choose JPEG, PNG, WebP, TIFF, HEIC, or BMP for image export.")?;
    Ok(ExportOptions {
        format,
        quality: 0.92,
        lossless: true,
    })
}
/// True when Windows can open HEIC files. The HEIF Image Extension alone is
/// not enough: decoding also needs an HEVC decoder (HEVC Video Extensions),
/// which Media Foundation lists. https://learn.microsoft.com/windows/win32/wic/heif-codec
pub fn heic_decode_available() -> bool {
    unsafe {
        factory().is_ok_and(|f| {
            f.CreateDecoder(&GUID_ContainerFormatHeif, std::ptr::null())
                .is_ok()
        }) && hevc_decoders() > 0
    }
}
/// True when Windows can save HEIC. This encodes a small test image: on the
/// dev PC, Media Foundation listed 3 hardware HEVC encoders, yet the HEIF
/// encoder failed with 0xC00D5212 because HEVC Video Extensions are missing.
#[allow(dead_code)] // The shell calls this in wave 2.
pub fn heic_encode_available() -> bool {
    unsafe {
        let Ok(factory) = factory() else {
            return false;
        };
        let Ok(bitmap) =
            factory.CreateBitmap(16, 16, &GUID_WICPixelFormat24bppBGR, WICBitmapCacheOnLoad)
        else {
            return false;
        };
        let Some(stream) = SHCreateMemStream(None) else {
            return false;
        };
        let options = ExportOptions {
            format: ImageFormat::Heic,
            quality: 0.5,
            lossless: false,
        };
        bitmap
            .cast()
            .is_ok_and(|source| encode_wic(&factory, &source, &stream, &options).is_ok())
    }
}
/// Count the HEVC decoders that Media Foundation lists.
/// https://learn.microsoft.com/windows/win32/api/mfapi/nf-mfapi-mftenumex
unsafe fn hevc_decoders() -> u32 {
    let input = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_HEVC,
    };
    let (mut list, mut count) = (std::ptr::null_mut(), 0u32);
    let category = MFT_CATEGORY_VIDEO_DECODER;
    if MFTEnumEx(
        category,
        MFT_ENUM_FLAG_ALL,
        Some(&input),
        None,
        &mut list,
        &mut count,
    )
    .is_err()
    {
        return 0;
    }
    for i in 0..count as usize {
        drop((*list.add(i)).take());
    }
    CoTaskMemFree(Some(list as *const _));
    count
}
const HEIC_HELP: &str = "Install HEIF Image Extensions and HEVC Video Extensions from the Microsoft Store, then try again.";
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
    options: &ExportOptions,
) -> Result<u64, String> {
    if output.exists() {
        return Err("A file already exists there. Choose a new name to keep both files.".into());
    }
    if format_for(output) != Some(options.format) {
        return Err("The file name extension does not match the export format.".into());
    }
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
        encode(factory, source, &stream, options)?;
    }
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&temp.0)
        .and_then(|file| file.sync_all())
        .map_err(|e| e.to_string())?;
    let written = if options.format == ImageFormat::WebP {
        let file = std::fs::File::open(&temp.0).map_err(|e| e.to_string())?;
        image_webp::WebPDecoder::new(std::io::BufReader::new(file))
            .map_err(|e| e.to_string())?
            .dimensions()
    } else {
        let check_path = wide(&temp.0);
        let check = factory
            .CreateDecoderFromFilename(
                PCWSTR(check_path.as_ptr()),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )
            .map_err(err)?;
        let frame = check.GetFrame(0).map_err(err)?;
        size(&frame)?
    };
    if written != size(source)? {
        return Err("The exported image dimensions did not match. No output was saved.".into());
    }
    let from = wide(&temp.0);
    let to = wide(output);
    MoveFileExW(
        PCWSTR(from.as_ptr()),
        PCWSTR(to.as_ptr()),
        MOVE_FILE_FLAGS(0),
    )
    .map_err(err)?;
    std::fs::metadata(output)
        .map(|m| m.len())
        .map_err(|e| e.to_string())
}
/// Encode `source` into `stream` in the chosen format.
unsafe fn encode(
    factory: &IWICImagingFactory,
    source: &IWICBitmapSource,
    stream: &IStream,
    options: &ExportOptions,
) -> Result<(), String> {
    if !(0.0..=1.0).contains(&options.quality) {
        return Err("Choose a quality from 0 to 100 percent.".into());
    }
    if options.format == ImageFormat::WebP {
        let bytes = webp_bytes(factory, source, options)?;
        let length = u32::try_from(bytes.len()).map_err(|e| e.to_string())?;
        let mut written = 0;
        stream
            .Write(bytes.as_ptr().cast(), length, Some(&mut written))
            .ok()
            .map_err(err)?;
        return if written == length {
            Ok(())
        } else {
            Err("Could not write the whole image.".into())
        };
    }
    let result = encode_wic(factory, source, stream, options);
    if options.format == ImageFormat::Heic {
        result.map_err(|e| format!("Windows cannot save HEIC on this PC. {HEIC_HELP} ({e})"))
    } else {
        result
    }
}
unsafe fn encode_wic(
    factory: &IWICImagingFactory,
    source: &IWICBitmapSource,
    stream: &IStream,
    options: &ExportOptions,
) -> Result<(), String> {
    let (container, mut pixel_format) = match options.format {
        ImageFormat::Jpeg => (GUID_ContainerFormatJpeg, GUID_WICPixelFormat24bppBGR),
        ImageFormat::Heic => (GUID_ContainerFormatHeif, GUID_WICPixelFormat24bppBGR),
        ImageFormat::Png => (GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA),
        ImageFormat::Tiff => (GUID_ContainerFormatTiff, GUID_WICPixelFormat32bppBGRA),
        ImageFormat::Bmp => (GUID_ContainerFormatBmp, GUID_WICPixelFormat32bppBGRA),
        ImageFormat::WebP => return Err("WebP does not use a Windows encoder.".into()),
    };
    let encoder = factory
        .CreateEncoder(&container, std::ptr::null())
        .map_err(err)?;
    encoder
        .Initialize(stream, WICBitmapEncoderNoCache)
        .map_err(err)?;
    let (mut frame, mut bag) = (None, None);
    encoder.CreateNewFrame(&mut frame, &mut bag).map_err(err)?;
    // Option names: https://learn.microsoft.com/windows/win32/wic/-wic-creating-encoder
    if let Some(bag) = &bag {
        let mut value = VARIANT::default();
        let inner = &mut *value.Anonymous.Anonymous;
        let name = match options.format {
            ImageFormat::Jpeg | ImageFormat::Heic => {
                inner.vt = VT_R4;
                inner.Anonymous.fltVal = options.quality;
                Some("ImageQuality")
            }
            ImageFormat::Bmp => {
                inner.vt = VT_BOOL;
                inner.Anonymous.boolVal = VARIANT_TRUE;
                Some("EnableV5Header32bppBGRA")
            }
            _ => None,
        };
        if let Some(name) = name {
            let mut name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            let property = PROPBAG2 {
                pstrName: PWSTR(name.as_mut_ptr()),
                ..Default::default()
            };
            bag.Write(1, &property, &value).map_err(err)?;
        }
    }
    let frame = frame.ok_or("Windows did not create an output encoder.")?;
    frame.Initialize(bag.as_ref()).map_err(err)?;
    let (width, height) = size(source)?;
    frame.SetSize(width, height).map_err(err)?;
    frame.SetPixelFormat(&mut pixel_format).map_err(err)?;
    let converter = factory.CreateFormatConverter().map_err(err)?;
    if pixel_format == GUID_WICPixelFormat24bppBGR {
        // The output has no alpha: composite onto white, one band of rows at a time.
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
        let band = 64u32;
        let mut bgra = vec![0u8; frame_bytes(width, 1)? * band as usize];
        let mut bgr = vec![0u8; width as usize * 3 * band as usize];
        for top in (0..height).step_by(band as usize) {
            let rows = band.min(height - top);
            let count = width as usize * rows as usize;
            let rect = WICRect {
                X: 0,
                Y: top as i32,
                Width: width as i32,
                Height: rows as i32,
            };
            converter
                .CopyPixels(&rect, width * 4, &mut bgra[..count * 4])
                .map_err(err)?;
            for (input, output) in bgra[..count * 4]
                .chunks_exact(4)
                .zip(bgr[..count * 3].chunks_exact_mut(3))
            {
                let white = 255 - input[3];
                for channel in 0..3 {
                    output[channel] = input[channel].saturating_add(white);
                }
            }
            frame
                .WritePixels(rows, width * 3, &bgr[..count * 3])
                .map_err(err)?;
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
    encoder.Commit().map_err(err)
}
/// WIC has no WebP encoder. libwebp encodes lossy WebP; image-webp encodes lossless.
unsafe fn webp_bytes(
    factory: &IWICImagingFactory,
    source: &IWICBitmapSource,
    options: &ExportOptions,
) -> Result<Vec<u8>, String> {
    let (width, height) = size(source)?;
    if width > 16383 || height > 16383 {
        return Err(
            "WebP allows at most 16383 pixels on each side. Resize the image, then try again."
                .into(),
        );
    }
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
    if options.lossless {
        // image-webp: 221 ms and 11.7 MB for a 12 MP photo; libwebp took 13.6 s for 10.3 MB.
        let mut bytes = Vec::new();
        image_webp::WebPEncoder::new(&mut bytes)
            .encode(&pixels, width, height, image_webp::ColorType::Rgba8)
            .map_err(|e| e.to_string())?;
        return Ok(bytes);
    }
    let mut output = std::ptr::null_mut();
    // Simple encoding API: https://developers.google.com/speed/webp/docs/api#simple_encoding_api
    let written = libwebp_sys::WebPEncodeRGBA(
        pixels.as_ptr(),
        width as i32,
        height as i32,
        width as i32 * 4,
        options.quality * 100.0,
        &mut output,
    );
    if written == 0 || output.is_null() {
        return Err("Could not encode this image as WebP.".into());
    }
    let bytes = std::slice::from_raw_parts(output, written).to_vec();
    libwebp_sys::WebPFree(output.cast());
    Ok(bytes)
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
