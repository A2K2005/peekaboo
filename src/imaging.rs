use crate::model::{
    fit_size, frame_bytes, outline, rgba, AnnotationKind, BatchJob, BatchResize, BatchResult,
    ExportOptions, Frame, ImageEdit, ImageFormat, MarkFont, MarkStyle, NormRect,
};
use std::{
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Mutex,
    },
};
use windows::{
    core::{w, Interface, PCWSTR, PWSTR},
    Win32::{
        Foundation::{
            GlobalFree, GENERIC_READ, GENERIC_WRITE, HANDLE, HGLOBAL, HWND, VARIANT_TRUE,
        },
        Graphics::Gdi::{BITMAPV5HEADER, BI_BITFIELDS, LCS_GM_IMAGES},
        Graphics::Imaging::*,
        Media::MediaFoundation::{
            MFMediaType_Video, MFTEnumEx, MFVideoFormat_HEVC, MFT_CATEGORY_VIDEO_DECODER,
            MFT_ENUM_FLAG_ALL, MFT_REGISTER_TYPE_INFO,
        },
        Storage::FileSystem::{MoveFileExW, MOVE_FILE_FLAGS},
        System::{
            Com::{
                CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IStream,
                StructuredStorage::{PropVariantClear, PropVariantToUInt16, PROPBAG2, PROPVARIANT},
                CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, STREAM_SEEK_END, STREAM_SEEK_SET,
            },
            DataExchange::{
                CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable,
                OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
            },
            Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
            Ole::{CF_DIB, CF_DIBV5},
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
            ImageEdit::FlipVertical => rotated(factory, &source, WICBitmapTransformFlipVertical)?,
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
                annotated(factory, &source, kind, &points, &text, MarkStyle::default())?
            }
            ImageEdit::Mark {
                kind,
                points,
                text,
                style,
            } => annotated(factory, &source, kind, &points, &text, style)?,
            ImageEdit::Clear {
                rect,
                ellipse,
                outside,
                white,
            } => cleared(factory, &source, rect, ellipse, outside, white)?,
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
    // Indexed and other formats need a palette or a color context; skip them.
    let plain = [
        GUID_WICPixelFormat24bppBGR,
        GUID_WICPixelFormat32bppBGR,
        GUID_WICPixelFormat32bppBGRA,
        GUID_WICPixelFormat32bppPBGRA,
        GUID_WICPixelFormat8bppGray,
    ];
    if !plain.contains(&format) {
        return None;
    }
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
    style: MarkStyle,
) -> Result<IWICBitmapSource, String> {
    use windows::Win32::Graphics::{
        Direct2D::Common::*, Direct2D::*, DirectWrite::*, Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
    };
    let style = style.visible(kind);
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
    let color = |c: u32| {
        let [r, g, b, a] = rgba(c);
        D2D1_COLOR_F { r, g, b, a }
    };
    let line = if kind == AnnotationKind::Highlight {
        D2D1_COLOR_F {
            r: 1.0,
            g: 0.85,
            b: 0.0,
            a: 0.35,
        }
    } else {
        color(style.stroke)
    };
    let brush = target.CreateSolidColorBrush(&line, None).map_err(err)?;
    // A fill of alpha 0 paints nothing, so shapes always fill.
    let fill = target
        .CreateSolidColorBrush(&color(style.fill), None)
        .map_err(err)?;
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
    // A 2 point line is 0.3% of the longer side, and 12 point text 3% of the
    // width, as if the image were a page about 667 points long.
    let thickness = (width.max(height) as f32 * 0.0015 * style.width).max(1.0);
    let ellipse = D2D1_ELLIPSE {
        point: windows_numerics::Vector2 {
            X: (rect.left + rect.right) / 2.0,
            Y: (rect.top + rect.bottom) / 2.0,
        },
        radiusX: (rect.right - rect.left) / 2.0,
        radiusY: (rect.bottom - rect.top) / 2.0,
    };
    let lens =if kind == AnnotationKind::Loupe {
        let picture = target
            .CreateBitmapFromWicBitmap(
                &converter,
                Some(&D2D1_BITMAP_PROPERTIES {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96.0,
                    dpiY: 96.0,
                }),
            )
            .map_err(err)?;
        let lens = target
            .CreateBitmapBrush(&picture, None, None)
            .map_err(err)?;
        let center = ellipse.point;
        lens.SetTransform(&windows_numerics::Matrix3x2 {
            M11: LOUPE_POWER,
            M12: 0.0,
            M21: 0.0,
            M22: LOUPE_POWER,
            M31: center.X * (1.0 - LOUPE_POWER),
            M32: center.Y * (1.0 - LOUPE_POWER),
        });
        Some(lens)
    } else {
        None
    };
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
        AnnotationKind::Rectangle => {
            target.FillRectangle(&rect, &fill);
            target.DrawRectangle(&rect, &brush, thickness, None);
        }
        AnnotationKind::Ellipse => {
            target.FillEllipse(&ellipse, &fill);
            target.DrawEllipse(&ellipse, &brush, thickness, None);
        }
        AnnotationKind::Line => target.DrawLine(a, b, &brush, thickness, None),
        AnnotationKind::RoundedRectangle
        | AnnotationKind::Bubble
        | AnnotationKind::Star
        | AnnotationKind::Polygon => {
            let shape = outline(kind, points[0], points[points.len() - 1], width as f32 / height as f32)
                .unwrap_or_default();
            let geometry = d2d.CreatePathGeometry().map_err(err)?;
            let sink = geometry.Open().map_err(err)?;
            sink.BeginFigure(to_point(shape[0]), D2D1_FIGURE_BEGIN_FILLED);
            let rest: Vec<_> = shape[1..].iter().map(|p| to_point(*p)).collect();
            sink.AddLines(&rest);
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            sink.Close().map_err(err)?;
            target.FillGeometry(&geometry, &fill, None);
            target.DrawGeometry(&geometry, &brush, thickness, None);
        }
        AnnotationKind::Loupe => {
            let radius = ellipse.radiusX.min(ellipse.radiusY);
            let circle = D2D1_ELLIPSE {
                radiusX: radius,
                radiusY: radius,
                ..ellipse
            };
            if let Some(lens) = &lens {
                target.FillEllipse(&circle, lens);
            }
            target.DrawEllipse(&circle, &brush, thickness, None);
        }
        AnnotationKind::Mask => {
            let dim = target
                .CreateSolidColorBrush(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.5 }, None)
                .map_err(err)?;
            let (w, h) = (width as f32, height as f32);
            for band in [
                D2D_RECT_F { left: 0.0, top: 0.0, right: w, bottom: rect.top },
                D2D_RECT_F { left: 0.0, top: rect.bottom, right: w, bottom: h },
                D2D_RECT_F { left: 0.0, top: rect.top, right: rect.left, bottom: rect.bottom },
                D2D_RECT_F { left: rect.right, top: rect.top, right: w, bottom: rect.bottom },
            ] {
                target.FillRectangle(&band, &dim);
            }
        }
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
            let length = (width.max(height) as f32 * 0.025).max(8.0).max(thickness * 4.0);
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
            let family = match style.font {
                MarkFont::Sans => w!("Segoe UI"),
                MarkFont::Serif => w!("Georgia"),
                MarkFont::Mono => w!("Consolas"),
            };
            let format = dwrite
                .CreateTextFormat(
                    family,
                    None,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    (width as f32 * 0.0025 * style.size).max(12.0),
                    w!("en-us"),
                )
                .map_err(err)?;
            let bounds = D2D_RECT_F {
                left: a.X,
                top: a.Y,
                right: (a.X + width as f32 * 0.6).min(width as f32),
                bottom: height as f32,
            };
            if kind == AnnotationKind::Text && points.len() > 1 {
                target.FillRectangle(&rect, &fill);
            }
            let ink = target
                .CreateSolidColorBrush(&color(style.text), None)
                .map_err(err)?;
            let text: Vec<u16> = text.encode_utf16().collect();
            target.DrawText(
                &text,
                &format,
                &bounds,
                if kind == AnnotationKind::Text { &ink } else { &brush },
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }
    target.EndDraw(None, None).map_err(err)?;
    bitmap.cast().map_err(err)
}

// Known limit: the power is fixed; no handle changes it yet.
const LOUPE_POWER: f32 = 2.0;

unsafe fn cleared(
    factory: &IWICImagingFactory,
    source: &IWICBitmapSource,
    rect: NormRect,
    ellipse: bool,
    outside: bool,
    white: bool,
) -> Result<IWICBitmapSource, String> {
    if !rect.iter().all(|n| n.is_finite() && (0.0..=1.0).contains(n))
        || rect[0] >= rect[2]
        || rect[1] >= rect[3]
    {
        return Err("Select an area inside the image.".into());
    }
    let (width, height) = size(source)?;
    if width as u64 * height as u64 > 50_000_000 {
        return Err("Resize images above 50 megapixels before clearing an area.".into());
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
    {
        let lock = bitmap
            .Lock(std::ptr::null(), WICBitmapLockWrite.0 as u32)
            .map_err(err)?;
        let stride = lock.GetStride().map_err(err)? as usize;
        let (mut length, mut data) = (0, std::ptr::null_mut());
        lock.GetDataPointer(&mut length, &mut data).map_err(err)?;
        let pixels = std::slice::from_raw_parts_mut(data, length as usize);
        let (w, h) = (width as f32, height as f32);
        let (l, t, r, b) = (rect[0] * w, rect[1] * h, rect[2] * w, rect[3] * h);
        let (cx, cy, rx, ry) = ((l + r) / 2.0, (t + b) / 2.0, (r - l) / 2.0, (b - t) / 2.0);
        let value = if white { [255; 4] } else { [0; 4] };
        for y in 0..height as usize {
            let py = y as f32 + 0.5;
            for x in 0..width as usize {
                let px = x as f32 + 0.5;
                let inside = if ellipse {
                    ((px - cx) / rx).powi(2) + ((py - cy) / ry).powi(2) <= 1.0
                } else {
                    px >= l && px < r && py >= t && py < b
                };
                if inside != outside {
                    let i = y * stride + x * 4;
                    pixels[i..i + 4].copy_from_slice(&value);
                }
            }
        }
    }
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
        let profile = rgb_profile(&factory, path);
        write_source(&factory, &source, output, options, profile.as_deref())
    }
}
/// The source's embedded RGB color profile. Pixels are written without
/// color conversion, so the profile still describes them. Gray and CMYK
/// profiles are left out: every output is RGB.
unsafe fn rgb_profile(factory: &IWICImagingFactory, path: &Path) -> Option<Vec<u8>> {
    let filename = wide(path);
    let frame = factory
        .CreateDecoderFromFilename(
            PCWSTR(filename.as_ptr()),
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )
        .and_then(|decoder| decoder.GetFrame(0))
        .ok()?;
    let mut count = 0;
    frame.GetColorContexts(&mut [], &mut count).ok()?;
    let mut contexts = Vec::new();
    for _ in 0..count.min(8) {
        contexts.push(Some(factory.CreateColorContext().ok()?));
    }
    frame.GetColorContexts(&mut contexts, &mut count).ok()?;
    contexts.into_iter().flatten().find_map(|context| {
        if context.GetType().ok()? != WICColorContextProfile {
            return None;
        }
        let mut length = 0;
        context.GetProfileBytes(&mut [], &mut length).ok()?;
        let mut bytes = vec![0u8; length as usize];
        context.GetProfileBytes(&mut bytes, &mut length).ok()?;
        // ICC header bytes 16 to 19: the data color space.
        (bytes.get(16..20) == Some(b"RGB ".as_slice())).then_some(bytes)
    })
}
/// The bytes `export_with` would write. No file is written. Above a pixel
/// budget it encodes full-resolution row bands and scales the size up: a full
/// encode of a 24 MP photo took 0.3 s for JPEG, 1.3 s for PNG, 1.9 s for WebP.
pub fn estimate_size(
    path: &Path,
    edits: &[ImageEdit],
    options: &ExportOptions,
) -> Result<u64, String> {
    unsafe {
        let factory = factory()?;
        let (source, _) = edited_source(&factory, path, edits, None)?;
        let (width, height) = size(&source)?;
        if options.format == ImageFormat::WebP {
            webp_length(width, height)?;
        }
        let profile = rgb_profile(&factory, path);
        let profile = profile.as_deref();
        let budget = match options.format {
            ImageFormat::Jpeg | ImageFormat::Bmp => 40_000_000,
            _ => 4_000_000,
        };
        if width as u64 * height as u64 <= budget {
            return encoded_length(&factory, &source, options, profile);
        }
        let (sample, rows) = row_sample(&factory, &source, width, height)?;
        let length = encoded_length(&factory, &sample, options, profile)?;
        Ok((length as f64 * height as f64 / rows as f64).round() as u64)
    }
}
unsafe fn encoded_length(
    factory: &IWICImagingFactory,
    source: &IWICBitmapSource,
    options: &ExportOptions,
    profile: Option<&[u8]>,
) -> Result<u64, String> {
    // SHCreateMemStream: a TIFF took 0.8 s here, CreateStreamOnHGlobal took 29.6 s.
    let stream = SHCreateMemStream(None).ok_or("Not enough memory to estimate the file size.")?;
    encode(factory, source, &stream, options, profile)?;
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
        write_source(&factory, &bitmap, output, &default_options(output)?, None).map(|_| ())
    }
}
/// Scale `frame` to `width` x `height` with WIC's Fant filter.
pub fn resize_frame(frame: &Frame, width: u32, height: u32) -> Result<Frame, String> {
    let mut pixels = vec![0; frame_bytes(width, height)?];
    unsafe {
        let factory = factory()?;
        let bitmap = frame_bitmap(&factory, frame)?;
        let scaler = factory.CreateBitmapScaler().map_err(err)?;
        scaler
            .Initialize(&bitmap, width, height, WICBitmapInterpolationModeFant)
            .map_err(err)?;
        scaler
            .CopyPixels(std::ptr::null(), width * 4, &mut pixels)
            .map_err(err)?;
    }
    Ok(Frame {
        width,
        height,
        pixels,
        page_count: 1,
        source_width: width,
        source_height: height,
    })
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
pub fn format_for(path: &Path) -> Option<ImageFormat> {
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
            .is_ok_and(|source| encode_wic(&factory, &source, &stream, &options, None).is_ok())
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
    profile: Option<&[u8]>,
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
        encode(factory, source, &stream, options, profile)?;
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
    profile: Option<&[u8]>,
) -> Result<(), String> {
    if !(0.0..=1.0).contains(&options.quality) {
        return Err("Choose a quality from 0 to 100 percent.".into());
    }
    if options.format == ImageFormat::WebP {
        let bytes = webp_bytes(factory, source, options, profile)?;
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
    let result = encode_wic(factory, source, stream, options, profile);
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
    profile: Option<&[u8]>,
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
    if let Some(profile) = profile {
        // An encoder that cannot store a profile refuses it; the image still saves.
        if let Ok(context) = factory.CreateColorContext() {
            if context.InitializeFromMemory(profile).is_ok() {
                let _ = frame.SetColorContexts(&[Some(context)]);
            }
        }
    }
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
fn webp_length(width: u32, height: u32) -> Result<usize, String> {
    if width > 16383 || height > 16383 {
        return Err(
            "WebP allows at most 16383 pixels on each side. Resize the image, then try again."
                .into(),
        );
    }
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .filter(|n| *n <= 256 * 1024 * 1024)
        .ok_or_else(|| "Resize images above 64 megapixels before exporting WebP.".into())
}

/// WIC has no WebP encoder. libwebp encodes lossy WebP; image-webp encodes lossless.
unsafe fn webp_bytes(
    factory: &IWICImagingFactory,
    source: &IWICBitmapSource,
    options: &ExportOptions,
    profile: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    let (width, height) = size(source)?;
    let length = webp_length(width, height)?;
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
        let mut encoder = image_webp::WebPEncoder::new(&mut bytes);
        if let Some(profile) = profile {
            encoder.set_icc_profile(profile.to_vec());
        }
        encoder
            .encode(&pixels, width, height, image_webp::ColorType::Rgba8)
            .map_err(|e| e.to_string())?;
        return Ok(bytes);
    }
    // ponytail: lossy WebP drops the color profile; libwebp's WebPMux API can add it.
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
/// Read an image from the clipboard. PNG comes first because it keeps alpha
/// reliably, then CF_DIBV5, then CF_DIB.
pub fn clipboard_image() -> Result<Frame, String> {
    unsafe {
        OpenClipboard(None).map_err(|_| "Another app is using the clipboard. Try again.")?;
        let read = || -> Result<(Vec<u8>, bool), String> {
            let png = RegisterClipboardFormatW(w!("PNG"));
            for (format, dib) in [
                (png, false),
                (CF_DIBV5.0 as u32, true),
                (CF_DIB.0 as u32, true),
            ] {
                if format != 0 && IsClipboardFormatAvailable(format).is_ok() {
                    let handle = GetClipboardData(format).map_err(err)?;
                    return Ok((global_bytes(HGLOBAL(handle.0))?, dib));
                }
            }
            Err("The clipboard has no image. Copy an image, then try again.".into())
        };
        let data = read();
        let _ = CloseClipboard();
        let (bytes, dib) = data?;
        decode_clipboard(&bytes, dib)
    }
}
/// Put the bytes from `encode_clipboard` on the clipboard: CF_DIBV5 and PNG,
/// both with alpha. `owner` is the app window: SetClipboardData fails when
/// no window owns the clipboard.
pub fn put_clipboard(owner: HWND, dib: &[u8], png: &[u8]) -> Result<(), String> {
    unsafe {
        OpenClipboard(Some(owner)).map_err(|_| "Another app is using the clipboard. Try again.")?;
        let write = || -> Result<(), String> {
            EmptyClipboard().map_err(err)?;
            set_global(CF_DIBV5.0 as u32, dib)?;
            set_global(RegisterClipboardFormatW(w!("PNG")), png)
        };
        let result = write();
        let _ = CloseClipboard();
        result
    }
}
unsafe fn global_bytes(handle: HGLOBAL) -> Result<Vec<u8>, String> {
    let data = GlobalLock(handle) as *const u8;
    if data.is_null() {
        return Err("Could not read the clipboard image.".into());
    }
    let bytes = std::slice::from_raw_parts(data, GlobalSize(handle)).to_vec();
    let _ = GlobalUnlock(handle);
    Ok(bytes)
}
unsafe fn set_global(format: u32, bytes: &[u8]) -> Result<(), String> {
    let handle = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).map_err(err)?;
    let data = GlobalLock(handle) as *mut u8;
    if data.is_null() {
        let _ = GlobalFree(Some(handle));
        return Err("Not enough memory to copy the image.".into());
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
    let _ = GlobalUnlock(handle);
    // After a successful SetClipboardData, the system owns the memory.
    if let Err(e) = SetClipboardData(format, Some(HANDLE(handle.0))) {
        let _ = GlobalFree(Some(handle));
        return Err(err(e));
    }
    Ok(())
}
/// Decode clipboard bytes: a PNG, or a packed DIB (`dib`) from CF_DIB or CF_DIBV5.
pub fn decode_clipboard(bytes: &[u8], dib: bool) -> Result<Frame, String> {
    if !dib {
        return unsafe { decode_memory(bytes) };
    }
    let file = bmp_file(bytes)?;
    let frame = unsafe { decode_memory(&file) }?;
    // Many apps write 32-bit DIBs with every alpha byte 0. Treat those as opaque.
    if frame.pixels.chunks_exact(4).all(|p| p[3] == 0) {
        return unsafe { decode_memory(&opaque_bmp(file)) };
    }
    Ok(frame)
}
/// A packed DIB with the 14-byte BITMAPFILEHEADER in front, as WIC expects.
/// https://learn.microsoft.com/windows/win32/gdi/bitmap-storage
fn bmp_file(dib: &[u8]) -> Result<Vec<u8>, String> {
    let read = |at: usize, n: usize| -> Option<u32> {
        let bytes = dib.get(at..at + n)?;
        Some(bytes.iter().rev().fold(0, |v, &b| v << 8 | b as u32))
    };
    let damaged = || "The clipboard image is damaged.".to_string();
    let header = read(0, 4).ok_or_else(damaged)? as usize;
    let (bits, compression, used, entry) = if header == 12 {
        (read(10, 2).ok_or_else(damaged)?, 0, 0, 3)
    } else if header >= 40 {
        let bits = read(14, 2).ok_or_else(damaged)?;
        let compression = read(16, 4).ok_or_else(damaged)?;
        (bits, compression, read(32, 4).ok_or_else(damaged)?, 4)
    } else {
        return Err(damaged());
    };
    let colors = match (used, bits) {
        (0, 1..=8) => 1 << bits,
        (n, _) => n as usize,
    };
    // BI_BITFIELDS (3) and BI_ALPHABITFIELDS (6) put masks after a 40-byte header.
    let masks = match (header, compression) {
        (40, 3) => 12,
        (40, 6) => 16,
        _ => 0,
    };
    let offset = 14 + header + masks + colors * entry;
    if colors > 256 || offset > 14 + dib.len() {
        return Err(damaged());
    }
    let size = u32::try_from(14 + dib.len()).map_err(|_| damaged())?;
    let mut file = Vec::with_capacity(14 + dib.len());
    file.extend_from_slice(b"BM");
    file.extend_from_slice(&size.to_le_bytes());
    file.extend_from_slice(&[0; 4]);
    file.extend_from_slice(&(offset as u32).to_le_bytes());
    file.extend_from_slice(dib);
    Ok(file)
}
/// The same BMP read as BI_RGB with a BITMAPINFOHEADER, so WIC ignores alpha.
/// The file header still points at the pixels, so the shorter header is safe.
fn opaque_bmp(mut file: Vec<u8>) -> Vec<u8> {
    file[14..18].copy_from_slice(&40u32.to_le_bytes());
    file[30..34].copy_from_slice(&0u32.to_le_bytes());
    file
}
unsafe fn decode_memory(bytes: &[u8]) -> Result<Frame, String> {
    let factory = factory()?;
    let stream = factory.CreateStream().map_err(err)?;
    stream.InitializeFromMemory(bytes).map_err(err)?;
    let frame = factory
        .CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnLoad)
        .and_then(|decoder| decoder.GetFrame(0))
        .map_err(|e| format!("Could not read the clipboard image. {e}"))?;
    let (width, height) = size(&frame)?;
    let mut pixels = vec![0; frame_bytes(width, height)?];
    let converter = factory.CreateFormatConverter().map_err(err)?;
    converter
        .Initialize(
            &frame,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )
        .map_err(err)?;
    converter
        .CopyPixels(std::ptr::null(), width * 4, &mut pixels)
        .map_err(err)?;
    Ok(Frame {
        width,
        height,
        pixels,
        page_count: 1,
        source_width: width,
        source_height: height,
    })
}
/// Clipboard bytes for `frame`: a CF_DIBV5 (bottom-up, straight alpha) and a PNG.
pub fn encode_clipboard(frame: &Frame) -> Result<(Vec<u8>, Vec<u8>), String> {
    let png = unsafe {
        let factory = factory()?;
        let bitmap = frame_bitmap(&factory, frame)?;
        let stream = SHCreateMemStream(None).ok_or("Not enough memory to copy the image.")?;
        let options = ExportOptions {
            format: ImageFormat::Png,
            quality: 1.0,
            lossless: true,
        };
        encode(&factory, &bitmap, &stream, &options, None)?;
        let mut length = 0;
        stream
            .Seek(0, STREAM_SEEK_END, Some(&mut length))
            .map_err(err)?;
        stream.Seek(0, STREAM_SEEK_SET, None).map_err(err)?;
        let mut png = vec![0u8; length as usize];
        let mut read = 0;
        stream
            .Read(png.as_mut_ptr().cast(), png.len() as u32, Some(&mut read))
            .ok()
            .map_err(err)?;
        if read as usize != png.len() {
            return Err("Could not copy the image.".into());
        }
        png
    };
    // https://learn.microsoft.com/windows/win32/api/wingdi/ns-wingdi-bitmapv5header
    let header = BITMAPV5HEADER {
        bV5Size: std::mem::size_of::<BITMAPV5HEADER>() as u32,
        bV5Width: frame.width as i32,
        bV5Height: frame.height as i32,
        bV5Planes: 1,
        bV5BitCount: 32,
        bV5Compression: BI_BITFIELDS,
        bV5SizeImage: frame.pixels.len() as u32,
        bV5RedMask: 0x00ff_0000,
        bV5GreenMask: 0x0000_ff00,
        bV5BlueMask: 0x0000_00ff,
        bV5AlphaMask: 0xff00_0000,
        bV5CSType: 0x7352_4742, // LCS_sRGB
        bV5Intent: LCS_GM_IMAGES as u32,
        ..Default::default()
    };
    let mut dib = Vec::with_capacity(header.bV5Size as usize + frame.pixels.len());
    dib.extend_from_slice(unsafe {
        std::slice::from_raw_parts(
            (&header as *const BITMAPV5HEADER).cast::<u8>(),
            header.bV5Size as usize,
        )
    });
    for row in frame.pixels.chunks_exact(frame.width as usize * 4).rev() {
        for p in row.chunks_exact(4) {
            let a = p[3] as u32;
            let straight = |c: u8| match a {
                0 => 0,
                _ => ((c as u32 * 255 + a / 2) / a).min(255) as u8,
            };
            dib.extend_from_slice(&[straight(p[0]), straight(p[1]), straight(p[2]), p[3]]);
        }
    }
    Ok((dib, png))
}
/// Apply one job to many files and write each result as a new file in
/// `output_dir`. Never overwrites: a taken name gets " (2)", " (3)", and so on.
/// Up to 4 files run at once. `progress(done, total)` runs after each file,
/// from a worker thread. Setting `cancel` skips the files not yet started.
/// Returns one result per input, in input order.
pub fn batch(
    inputs: &[PathBuf],
    output_dir: &Path,
    job: &BatchJob,
    progress: &(dyn Fn(usize, usize) + Sync),
    cancel: &AtomicBool,
) -> Vec<BatchResult> {
    let total = inputs.len();
    if let Err(e) = std::fs::create_dir_all(output_dir) {
        let output = Err(format!("Could not use the output folder. {e}"));
        return inputs
            .iter()
            .map(|input| BatchResult {
                input: input.clone(),
                output: output.clone(),
            })
            .collect();
    }
    let mut taken = std::collections::HashSet::new();
    let plans: Vec<_> = inputs
        .iter()
        .map(|input| batch_plan(input, output_dir, job, &mut taken))
        .collect();
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let results = Mutex::new(vec![None; total]);
    let workers = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .clamp(1, 4)
        .min(total);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= total {
                        break;
                    }
                    let (output, options) = &plans[i];
                    let result = if cancel.load(Ordering::Relaxed) {
                        Err("Canceled.".to_string())
                    } else {
                        unsafe { batch_one(&inputs[i], output, job, options) }
                            .map(|_| output.clone())
                    };
                    if let Ok(mut results) = results.lock() {
                        results[i] = Some(result);
                    }
                    progress(done.fetch_add(1, Ordering::Relaxed) + 1, total);
                }
                if com.is_ok() {
                    unsafe { CoUninitialize() };
                }
            });
        }
    });
    let results = results.into_inner().unwrap_or_default();
    inputs
        .iter()
        .zip(results)
        .map(|(input, output)| BatchResult {
            input: input.clone(),
            output: output.unwrap_or_else(|| Err("Not processed.".into())),
        })
        .collect()
}
/// Pick the output format and a free output name for one batch input.
fn batch_plan(
    input: &Path,
    output_dir: &Path,
    job: &BatchJob,
    taken: &mut std::collections::HashSet<PathBuf>,
) -> (PathBuf, ExportOptions) {
    let own = format_for(input);
    let options = job.options.unwrap_or(ExportOptions {
        format: own.unwrap_or(ImageFormat::Png),
        quality: 0.92,
        lossless: true,
    });
    let extension = match options.format {
        _ if own == Some(options.format) => input
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default(),
        ImageFormat::Jpeg => "jpg".into(),
        ImageFormat::Png => "png".into(),
        ImageFormat::WebP => "webp".into(),
        ImageFormat::Tiff => "tif".into(),
        ImageFormat::Heic => "heic".into(),
        ImageFormat::Bmp => "bmp".into(),
    };
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".into());
    let mut output = output_dir.join(format!("{stem}.{extension}"));
    let mut n = 2;
    while output.exists()
        || taken.contains(&PathBuf::from(
            output.as_os_str().to_string_lossy().to_lowercase(),
        ))
    {
        output = output_dir.join(format!("{stem} ({n}).{extension}"));
        n += 1;
    }
    taken.insert(PathBuf::from(
        output.as_os_str().to_string_lossy().to_lowercase(),
    ));
    (output, options)
}
unsafe fn batch_one(
    input: &Path,
    output: &Path,
    job: &BatchJob,
    options: &ExportOptions,
) -> Result<u64, String> {
    let factory = factory()?;
    let turns = vec![ImageEdit::RotateRight; (job.quarter_turns % 4) as usize];
    let (mut source, (width, height)) = edited_source(&factory, input, &turns, None)?;
    if let Some(resize) = job.resize {
        let (w, h) = resize_target(width, height, resize)?;
        if (w, h) != (width, height) {
            let scaler = factory.CreateBitmapScaler().map_err(err)?;
            scaler
                .Initialize(&source, w, h, WICBitmapInterpolationModeFant)
                .map_err(err)?;
            source = scaler.cast().map_err(err)?;
        }
    }
    let profile = rgb_profile(&factory, input);
    write_source(&factory, &source, output, options, profile.as_deref())
}
/// The size a batch resize gives a `width` x `height` image.
fn resize_target(width: u32, height: u32, resize: BatchResize) -> Result<(u32, u32), String> {
    let scaled = |scale: f64| {
        (
            ((width as f64 * scale).round() as u32).max(1),
            ((height as f64 * scale).round() as u32).max(1),
        )
    };
    let size = match resize {
        BatchResize::Pixels { width, height } => (width, height),
        BatchResize::Percent(p) if p.is_finite() && p > 0.0 => scaled(p as f64 / 100.0),
        BatchResize::MaxEdge(edge) if edge > 0 => {
            scaled((edge as f64 / width.max(height) as f64).min(1.0))
        }
        _ => return Err("Enter a size greater than 0.".into()),
    };
    validate_dimensions(size.0, size.1)?;
    Ok(size)
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
    fn webp_limits_apply_before_sampling() {
        assert!(webp_length(16384, 1).is_err());
        assert!(webp_length(1, 16384).is_err());
        assert!(webp_length(10000, 10000).is_err());
        assert_eq!(webp_length(16383, 1).unwrap(), 16383 * 4);
        assert_eq!(webp_length(8192, 8192).unwrap(), 256 * 1024 * 1024);
    }
    #[test]
    fn batch_names_ignore_case() {
        let mut taken = std::collections::HashSet::new();
        let job = BatchJob {
            quarter_turns: 0,
            resize: None,
            options: None,
        };
        let dir = Path::new("artifacts/not-created-case-test");
        let first = batch_plan(Path::new("Photo.PNG"), dir, &job, &mut taken).0;
        let second = batch_plan(Path::new("photo.png"), dir, &job, &mut taken).0;
        assert_eq!(first.file_name().unwrap(), "Photo.PNG");
        assert_eq!(second.file_name().unwrap(), "photo (2).png");
    }
    #[test]
    fn crop_bounds() {
        let r = crop_rect(100, 200, 0.1, 0.2, 0.8, 0.9).unwrap();
        assert_eq!((r.X, r.Y, r.Width, r.Height), (10, 40, 70, 140));
        assert!(crop_rect(100, 100, f32::NAN, 0.0, 1.0, 1.0).is_err());
        assert!(crop_rect(100, 100, 0.8, 0.0, 0.2, 1.0).is_err());
        assert!(crop_rect(100, 100, -0.1, 0.0, 1.0, 1.0).is_err());
    }
    #[test]
    fn batch_resize_sizes() {
        use BatchResize::*;
        assert_eq!(resize_target(4000, 3000, Percent(50.0)), Ok((2000, 1500)));
        assert_eq!(resize_target(3000, 4000, MaxEdge(1000)), Ok((750, 1000)));
        assert_eq!(resize_target(800, 600, MaxEdge(1000)), Ok((800, 600)));
        let exact = Pixels {
            width: 10,
            height: 20,
        };
        assert_eq!(resize_target(800, 600, exact), Ok((10, 20)));
        assert_eq!(resize_target(3, 1, Percent(10.0)), Ok((1, 1)));
        for bad in [Percent(0.0), Percent(f32::NAN), MaxEdge(0), Percent(1e9)] {
            assert!(resize_target(800, 600, bad).is_err());
        }
    }
}
