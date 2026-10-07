#![allow(dead_code)]
//! Timing runs. They print numbers and are ignored by default.
//! Run: cargo test --release --test image_perf -- --ignored --nocapture --test-threads=1
//! Set PFW_PHOTOS to a folder with 24mp_landscape.jpg and 4000 x 3000 photos.
#[path = "../src/imaging.rs"]
mod imaging;
#[path = "../src/model.rs"]
mod model;

use std::{path::PathBuf, time::Instant};

fn com() {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap() };
}
fn photos() -> PathBuf {
    std::env::var_os("PFW_PHOTOS")
        .map(PathBuf::from)
        .expect("Set PFW_PHOTOS to a folder of real photos")
}
/// The JPEG photos whose names start with a digit, sorted.
fn photo_list() -> Vec<PathBuf> {
    let mut list: Vec<PathBuf> = std::fs::read_dir(photos())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            name.ends_with(".jpg") && name.starts_with(|c: char| c.is_ascii_digit())
        })
        .collect();
    list.sort();
    list
}
fn times(label: &str, runs: usize, mut f: impl FnMut()) -> Vec<f64> {
    f();
    let mut ms: Vec<f64> = (0..runs)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    let all = ms
        .iter()
        .map(|t| format!("{t:.1}"))
        .collect::<Vec<_>>()
        .join(", ");
    ms.sort_by(f64::total_cmp);
    println!("{label}: median {:.1} ms [{all}]", ms[ms.len() / 2]);
    ms
}

#[test]
#[ignore = "Timing run"]
fn display_decode_24mp() {
    com();
    let path = photos().join("24mp_landscape.jpg");
    times("decode 24 MP to 1920x1080", 7, || {
        let f = imaging::decode(&path, 1920, 1080).unwrap();
        assert_eq!((f.source_width, f.source_height), (6000, 4000));
    });
}

/// Where the display decode time goes: native JPEG scale factors and the scaler.
#[test]
#[ignore = "Timing run"]
fn display_decode_breakdown() {
    use windows::{
        core::{Interface, HSTRING},
        Win32::{Foundation::GENERIC_READ, Graphics::Imaging::*, System::Com::*},
    };
    com();
    let path = HSTRING::from(photos().join("24mp_landscape.jpg").as_os_str());
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).unwrap();
        let open = || {
            factory
                .CreateDecoderFromFilename(
                    &path,
                    None,
                    GENERIC_READ,
                    WICDecodeMetadataCacheOnDemand,
                )
                .unwrap()
                .GetFrame(0)
                .unwrap()
        };
        times("open decoder and frame", 7, || drop(open()));
        for (w, h) in [(6000u32, 4000u32), (3000, 2000), (1500, 1000), (750, 500)] {
            let mut buf = vec![0u8; (w * h * 3) as usize];
            times(&format!("native decode {w}x{h} 24bpp"), 7, || {
                let t: IWICBitmapSourceTransform = open().cast().unwrap();
                t.CopyPixels(
                    std::ptr::null(),
                    w,
                    h,
                    &GUID_WICPixelFormat24bppBGR,
                    WICBitmapTransformRotate0,
                    w * 3,
                    &mut buf,
                )
                .unwrap();
            });
        }
        let half = vec![0u8; 3000 * 2000 * 3];
        let mut out = vec![0u8; 1620 * 1080 * 4];
        times("Fant 3000x2000 to 1620x1080 plus PBGRA", 7, || {
            let b = factory
                .CreateBitmapFromMemory(3000, 2000, &GUID_WICPixelFormat24bppBGR, 9000, &half)
                .unwrap();
            let s = factory.CreateBitmapScaler().unwrap();
            s.Initialize(&b, 1620, 1080, WICBitmapInterpolationModeFant)
                .unwrap();
            let c = factory.CreateFormatConverter().unwrap();
            c.Initialize(
                &s,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
            .unwrap();
            c.CopyPixels(std::ptr::null(), 1620 * 4, &mut out).unwrap();
        });
    }
}

/// Estimate time on the 24 MP photo, and estimate error against real exports
/// for every photo in PFW_PHOTOS.
#[test]
#[ignore = "Timing run"]
fn estimate_speed_and_accuracy() {
    use model::{ExportOptions, ImageFormat::*};
    com();
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts/estimate");
    std::fs::create_dir_all(&out).unwrap();
    let formats = [
        (Jpeg, false, "jpg"),
        (Png, false, "png"),
        (WebP, false, "webp"),
        (WebP, true, "webp"),
        (Tiff, false, "tif"),
        (Bmp, false, "bmp"),
    ];
    let big = photos().join("24mp_landscape.jpg");
    for (format, lossless, _) in formats {
        let options = ExportOptions {
            format,
            quality: 0.8,
            lossless,
        };
        times(
            &format!("estimate 24 MP {format:?} lossless={lossless}"),
            5,
            || {
                imaging::estimate_size(&big, &[], &options).unwrap();
            },
        );
    }
    let photos = photo_list();
    for (format, lossless, ext) in formats {
        let options = ExportOptions {
            format,
            quality: 0.8,
            lossless,
        };
        let mut errors = Vec::new();
        for photo in &photos {
            let estimate = imaging::estimate_size(photo, &[], &options).unwrap() as f64;
            let target = out.join(format!("{}-{}.{ext}", std::process::id(), errors.len()));
            let written = imaging::export_with(photo, &target, &[], &options).unwrap() as f64;
            std::fs::remove_file(&target).unwrap();
            errors.push((estimate - written) / written * 100.0);
        }
        let worst = errors.iter().fold(0f64, |m, e| m.max(e.abs()));
        let mean = errors.iter().map(|e| e.abs()).sum::<f64>() / errors.len() as f64;
        let all = errors
            .iter()
            .map(|e| format!("{e:+.1}"))
            .collect::<Vec<_>>()
            .join(" ");
        println!(
            "{format:?} lossless={lossless}: mean |error| {mean:.1}%, worst {worst:.1}% [{all}]"
        );
    }
}

#[test]
#[ignore = "Timing run"]
fn rotate_export_24mp() {
    use model::{ExportOptions, ImageEdit, ImageFormat};
    com();
    let path = photos().join("24mp_landscape.jpg");
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts/rotate");
    std::fs::create_dir_all(&out).unwrap();
    let options = ExportOptions {
        format: ImageFormat::Jpeg,
        quality: 0.9,
        lossless: false,
    };
    let mut n = 0;
    times("export 24 MP rotated right as JPEG", 5, || {
        n += 1;
        let target = out.join(format!("{}-{n}.jpg", std::process::id()));
        imaging::export_with(&path, &target, &[ImageEdit::RotateRight], &options).unwrap();
        std::fs::remove_file(target).unwrap();
    });
}

#[test]
#[ignore = "Experiment"]
fn hevc_mft_experiment() {
    use windows::Win32::Media::MediaFoundation::*;
    com();
    unsafe {
        for (label, category, subtype, decoder) in [
            ("HEVC decoders", MFT_CATEGORY_VIDEO_DECODER, MFVideoFormat_HEVC, true),
            ("HEVC encoders", MFT_CATEGORY_VIDEO_ENCODER, MFVideoFormat_HEVC, false),
            ("AV1 decoders", MFT_CATEGORY_VIDEO_DECODER, MFVideoFormat_AV1, true),
        ] {
            let info = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: subtype };
            let (i, o) = if decoder { (Some(&info as *const _), None) } else { (None, Some(&info as *const _)) };
            let (mut list, mut count) = (std::ptr::null_mut(), 0u32);
            MFTEnumEx(category, MFT_ENUM_FLAG_ALL, i, o, &mut list, &mut count).unwrap();
            let mut names = Vec::new();
            for k in 0..count as usize {
                let a = (*list.add(k)).take().unwrap();
                let mut p = windows::core::PWSTR::null();
                let mut n = 0;
                if a.GetAllocatedString(&MFT_FRIENDLY_NAME_Attribute, &mut p, &mut n).is_ok() {
                    names.push(p.to_string().unwrap());
                }
            }
            println!("{label}: {count} {names:?}");
        }
    }
    println!("heic decode available: {}", imaging::heic_decode_available());
    println!("heic encode available: {}", imaging::heic_encode_available());
}
