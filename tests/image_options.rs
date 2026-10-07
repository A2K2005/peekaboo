#![allow(dead_code)]
#[path = "../src/imaging.rs"]
mod imaging;
#[path = "../src/model.rs"]
mod model;

use model::{ExportOptions, Frame, ImageEdit, ImageFormat};
use std::path::{Path, PathBuf};

fn com() {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap() };
}
/// PFW_FIXTURES overrides the fixtures folder.
fn fixture(name: &str) -> PathBuf {
    let root = std::env::var_os("PFW_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures"));
    let path = root.join(name);
    assert!(path.is_file(), "Missing fixture {}", path.display());
    path
}
fn out_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("artifacts")
        .join(name)
        .join(format!("{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
fn options(format: ImageFormat, quality: f32, lossless: bool) -> ExportOptions {
    ExportOptions {
        format,
        quality,
        lossless,
    }
}
fn mean_difference(a: &Frame, b: &Frame) -> f64 {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let total: u64 = a
        .pixels
        .iter()
        .zip(&b.pixels)
        .map(|(x, y)| x.abs_diff(*y) as u64)
        .sum();
    total as f64 / a.pixels.len() as f64
}

#[test]
fn export_with_options_controls_format_and_quality() {
    com();
    let out = out_dir("export-options");
    let photo = fixture("image-small.png");
    let original = imaging::decode(&photo, 640, 480).unwrap();
    let low = out.join("low.jpg");
    let high = out.join("high.jpg");
    let low_bytes =
        imaging::export_with(&photo, &low, &[], &options(ImageFormat::Jpeg, 0.2, false)).unwrap();
    let high_bytes =
        imaging::export_with(&photo, &high, &[], &options(ImageFormat::Jpeg, 0.95, false)).unwrap();
    assert_eq!(low_bytes, std::fs::metadata(&low).unwrap().len());
    assert!(low_bytes < high_bytes, "JPEG quality had no effect");

    let lossy = out.join("lossy.webp");
    let lossless = out.join("lossless.webp");
    let lossy_bytes = imaging::export_with(
        &photo,
        &lossy,
        &[],
        &options(ImageFormat::WebP, 0.75, false),
    )
    .unwrap();
    let lossless_bytes = imaging::export_with(
        &photo,
        &lossless,
        &[],
        &options(ImageFormat::WebP, 0.75, true),
    )
    .unwrap();
    assert_eq!(lossy_bytes, std::fs::metadata(&lossy).unwrap().len());
    assert_eq!(lossless_bytes, std::fs::metadata(&lossless).unwrap().len());
    // RIFF header, then the first chunk: "VP8 " is lossy, "VP8L" is lossless.
    assert_eq!(&std::fs::read(&lossy).unwrap()[12..16], b"VP8 ");
    assert_eq!(&std::fs::read(&lossless).unwrap()[12..16], b"VP8L");
    let decoded = imaging::decode(&lossy, 640, 480).unwrap();
    assert!(
        mean_difference(&decoded, &original) < 4.0,
        "Lossy WebP is wrong"
    );
    assert_eq!(
        imaging::decode(&lossless, 640, 480).unwrap().pixels,
        original.pixels,
        "Lossless WebP changed pixels"
    );
    // The fixture is flat color. Rendered text gives the encoder detail to drop.
    let detailed = out.join("detailed.png");
    let text = ImageEdit::Annotate {
        kind: model::AnnotationKind::Text,
        points: vec![[0.02, 0.02]],
        text: "Preview for Windows exports WebP with a quality setting. ".repeat(12),
    };
    imaging::export(&photo, &detailed, &[text]).unwrap();
    let sizes: Vec<u64> = [0.1, 0.9]
        .iter()
        .map(|&quality| {
            let path = out.join(format!("text-{quality}.webp"));
            imaging::export_with(
                &detailed,
                &path,
                &[],
                &options(ImageFormat::WebP, quality, false),
            )
            .unwrap()
        })
        .collect();
    assert!(sizes[0] < sizes[1], "WebP quality had no effect: {sizes:?}");

    // BMP keeps alpha through a BITMAPV5HEADER.
    let alpha = Frame {
        width: 2,
        height: 1,
        pixels: vec![0, 0, 128, 128, 0, 0, 0, 0],
        page_count: 1,
        source_width: 2,
        source_height: 1,
    };
    let alpha_png = out.join("alpha.png");
    imaging::export_frame(&alpha, &alpha_png).unwrap();
    let bmp = out.join("alpha.bmp");
    imaging::export_with(
        &alpha_png,
        &bmp,
        &[],
        &options(ImageFormat::Bmp, 1.0, false),
    )
    .unwrap();
    assert_eq!(imaging::decode(&bmp, 2, 1).unwrap().pixels, alpha.pixels);

    for format in [ImageFormat::Png, ImageFormat::Tiff] {
        let path = out.join(format!(
            "same.{}",
            if format == ImageFormat::Png {
                "png"
            } else {
                "tif"
            }
        ));
        imaging::export_with(&photo, &path, &[], &options(format, 0.5, false)).unwrap();
        assert_eq!(
            imaging::decode(&path, 640, 480).unwrap().pixels,
            original.pixels
        );
    }

    let mismatch = out.join("mismatch.png");
    assert!(imaging::export_with(
        &photo,
        &mismatch,
        &[],
        &options(ImageFormat::Jpeg, 0.9, false)
    )
    .is_err());
    assert!(!mismatch.exists());
    for quality in [-0.1, 1.5, f32::NAN] {
        let path = out.join("bad-quality.jpg");
        assert!(imaging::export_with(
            &photo,
            &path,
            &[],
            &options(ImageFormat::Jpeg, quality, false)
        )
        .is_err());
        assert!(!path.exists());
    }
    assert!(
        imaging::export_with(&photo, &low, &[], &options(ImageFormat::Jpeg, 0.5, false)).is_err(),
        "Export overwrote a file"
    );
}

#[test]
fn heic_export_matches_codec_availability() {
    com();
    let out = out_dir("heic");
    let target = out.join("photo.heic");
    let result = imaging::export_with(
        &fixture("image-small.png"),
        &target,
        &[],
        &options(ImageFormat::Heic, 0.8, false),
    );
    assert_eq!(result.is_ok(), imaging::heic_encode_available());
    match result {
        Ok(_) => assert!(imaging::heic_decode_available()),
        Err(e) => {
            assert!(e.contains("HEVC Video Extensions"), "{e}");
            assert!(!target.exists());
        }
    }
    // Without the HEVC decoder, opening a HEIC file says what to install.
    let fake = out.join("fake.heic");
    std::fs::write(&fake, b"not a heic file").unwrap();
    let error = imaging::decode(&fake, 100, 100).err().unwrap();
    assert_eq!(
        error.contains("HEVC Video Extensions"),
        !imaging::heic_decode_available(),
        "{error}"
    );
}

#[test]
fn estimate_matches_export() {
    com();
    let out = out_dir("estimate");
    let small = fixture("image-small.png");
    let large = fixture("image-24mp.jpg");
    let cases = [
        (ImageFormat::Jpeg, false, "jpg"),
        (ImageFormat::Png, false, "png"),
        (ImageFormat::WebP, false, "webp"),
        (ImageFormat::WebP, true, "webp"),
        (ImageFormat::Tiff, false, "tif"),
        (ImageFormat::Bmp, false, "bmp"),
    ];
    for (index, (format, lossless, extension)) in cases.into_iter().enumerate() {
        let options = options(format, 0.8, lossless);
        // Small images encode in full, so the estimate is exact.
        let estimate = imaging::estimate_size(&small, &[], &options).unwrap();
        let path = out.join(format!("small-{index}.{extension}"));
        assert_eq!(
            estimate,
            imaging::export_with(&small, &path, &[], &options).unwrap()
        );
        // 24 MP PNG, WebP, and TIFF estimates come from row bands.
        let estimate = imaging::estimate_size(&large, &[], &options).unwrap() as f64;
        let path = out.join(format!("large-{index}.{extension}"));
        let written = imaging::export_with(&large, &path, &[], &options).unwrap() as f64;
        let error = (estimate - written).abs() / written;
        // This flat-color fixture compresses to a few KB, so fixed per-band
        // costs weigh more: up to 22% here. On 13 real photos the worst was 7.4%.
        assert!(
            error < 0.25,
            "{format:?} estimate off by {:.1}%",
            error * 100.0
        );
    }
    // Edits change the estimate the same way they change the export.
    let edits = [ImageEdit::Resize {
        width: 320,
        height: 240,
    }];
    let options = options(ImageFormat::Png, 1.0, false);
    let path = out.join("resized.png");
    assert_eq!(
        imaging::estimate_size(&small, &edits, &options).unwrap(),
        imaging::export_with(&small, &path, &edits, &options).unwrap()
    );
}

/// Write a copy of `source` as a JPEG whose EXIF orientation is `orientation`.
fn oriented_jpeg(source: &Path, output: &Path, orientation: u16) {
    use windows::{
        core::{Interface, HSTRING},
        Win32::{
            Foundation::*,
            Graphics::Imaging::*,
            System::{Com::*, Variant::VT_UI2},
        },
    };
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).unwrap();
        let frame = factory
            .CreateDecoderFromFilename(
                &HSTRING::from(source.as_os_str()),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )
            .unwrap()
            .GetFrame(0)
            .unwrap();
        let stream = factory.CreateStream().unwrap();
        stream
            .InitializeFromFilename(&HSTRING::from(output.as_os_str()), GENERIC_WRITE.0)
            .unwrap();
        let encoder = factory
            .CreateEncoder(&GUID_ContainerFormatJpeg, std::ptr::null())
            .unwrap();
        encoder
            .Initialize(&stream, WICBitmapEncoderNoCache)
            .unwrap();
        let (mut target, mut bag) = (None, None);
        encoder.CreateNewFrame(&mut target, &mut bag).unwrap();
        let target = target.unwrap();
        target.Initialize(bag.as_ref()).unwrap();
        let mut value = StructuredStorage::PROPVARIANT::default();
        let inner = &mut *value.Anonymous.Anonymous;
        inner.vt = VT_UI2;
        inner.Anonymous.uiVal = orientation;
        target
            .GetMetadataQueryWriter()
            .unwrap()
            .SetMetadataByName(&HSTRING::from("/app1/ifd/{ushort=274}"), &value)
            .unwrap();
        target
            .WriteSource(&frame.cast::<IWICBitmapSource>().unwrap(), std::ptr::null())
            .unwrap();
        target.Commit().unwrap();
        encoder.Commit().unwrap();
    }
}

#[test]
fn display_decode_uses_native_scale_without_changing_the_picture() {
    com();
    let out = out_dir("display-decode");
    let photo = fixture("image-24mp.jpg");
    let rotated = out.join("rotated.jpg");
    oriented_jpeg(&photo, &rotated, 6);
    // A full-image crop disables native scaling, so it is the reference.
    let reference = [ImageEdit::Crop {
        left: 0.0,
        top: 0.0,
        right: 1.0,
        bottom: 1.0,
    }];
    for (path, size, full) in [
        (&photo, (1620, 1080), (6000, 4000)),
        (&rotated, (720, 1080), (4000, 6000)),
    ] {
        let fast = imaging::decode(path, 1920, 1080).unwrap();
        let slow = imaging::decode_edited(path, 1920, 1080, &reference).unwrap();
        assert_eq!((fast.width, fast.height), size);
        assert_eq!((fast.source_width, fast.source_height), full);
        let difference = mean_difference(&fast, &slow);
        assert!(
            difference < 1.5,
            "Native scaling changed the picture by {difference}"
        );
    }
    // Other formats take the native path only when their codec offers it.
    let small = fixture("image-small.png");
    for extension in ["tif", "bmp", "webp", "png"] {
        let path = out.join(format!("small.{extension}"));
        imaging::export(&small, &path, &[]).unwrap();
        let fast = imaging::decode(&path, 320, 240).unwrap();
        let slow = imaging::decode_edited(&path, 320, 240, &reference).unwrap();
        assert!(mean_difference(&fast, &slow) < 1.5, "{extension} changed");
    }
}

#[test]
fn webp_estimate_rejects_dimensions_that_export_cannot_encode() {
    com();
    let photo = fixture("image-small.png");
    for (width, height, message) in [(16384, 400, "16383"), (10000, 10000, "64 megapixels")] {
        let error = imaging::estimate_size(&photo, &[ImageEdit::Resize { width, height }],
            &options(ImageFormat::WebP, 0.8, false)).unwrap_err();
        assert!(error.contains(message), "{error}");
    }
}
