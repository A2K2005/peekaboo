#![allow(dead_code)]
#[path = "../src/imaging.rs"]
mod imaging;
#[path = "../src/model.rs"]
mod model;

#[test]
fn image_exports_preserve_resolution_alpha_and_source() {
    use model::{Frame, ImageEdit};
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap() };
    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe { CoUninitialize() }
        }
    }
    let _com = Com;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = root.join("artifacts/image-workflow").join(format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&out).unwrap();
    let source = out.join("alpha.png");
    let pixels = vec![255, 0, 0, 255, 0, 0, 128, 128, 0, 0, 0, 0, 0, 255, 0, 255];
    let frame = Frame {
        width: 2,
        height: 2,
        pixels: pixels.clone(),
        page_count: 1,
        source_width: 2,
        source_height: 2,
    };
    imaging::export_frame(&frame, &source).unwrap();
    let original = fs::read(&source).unwrap();
    assert_eq!(
        imaging::decode(&source, 10, 10).unwrap().pixels,
        pixels,
        "PNG lost premultiplied alpha or pixel values"
    );
    let webp = out.join("alpha.webp");
    imaging::export(&source, &webp, &[]).unwrap();
    let webp_frame = imaging::decode(&webp, 10, 10).unwrap();
    assert_eq!((webp_frame.width, webp_frame.height), (2, 2));
    assert_eq!(
        webp_frame.pixels, pixels,
        "Lossless WebP roundtrip changed premultiplied pixels"
    );
    assert!(
        imaging::export(&source, &webp, &[]).is_err(),
        "WebP overwrote an existing output"
    );
    let transparent = Frame {
        width: 32,
        height: 32,
        pixels: vec![0; 32 * 32 * 4],
        page_count: 1,
        source_width: 32,
        source_height: 32,
    };
    let jpeg = out.join("transparent-white.jpg");
    imaging::export_frame(&transparent, &jpeg).unwrap();
    let white = imaging::decode(&jpeg, 32, 32).unwrap();
    assert!(
        white
            .pixels
            .chunks_exact(4)
            .all(|p| p[0] >= 250 && p[1] >= 250 && p[2] >= 250 && p[3] == 255),
        "JPEG transparency was not composited onto white"
    );
    let rotated = out.join("rotated.png");
    imaging::export(&source, &rotated, &[ImageEdit::RotateRight]).unwrap();
    let actual = imaging::decode(&rotated, 10, 10).unwrap();
    assert_eq!(
        actual.pixels,
        vec![0, 0, 0, 0, 255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 128, 128]
    );
    let crop = out.join("crop.png");
    imaging::export(
        &source,
        &crop,
        &[ImageEdit::Crop {
            left: 0.0,
            top: 0.0,
            right: 0.5,
            bottom: 1.0,
        }],
    )
    .unwrap();
    let cropped = imaging::decode(&crop, 10, 10).unwrap();
    assert_eq!((cropped.width, cropped.height), (1, 2));
    assert_eq!(cropped.pixels, vec![255, 0, 0, 255, 0, 0, 0, 0]);
    let resize = out.join("resize.png");
    imaging::export(
        &source,
        &resize,
        &[ImageEdit::Resize {
            width: 4,
            height: 6,
        }],
    )
    .unwrap();
    let resized = imaging::decode(&resize, 10, 10).unwrap();
    assert_eq!((resized.width, resized.height), (4, 6));
    // PFW_FIXTURES overrides the fixtures folder.
    let fixtures = std::env::var_os("PFW_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("fixtures"));
    let large = fixtures.join("image-24mp.jpg");
    assert!(large.is_file(), "Run tools/make-fixtures.ps1 first");
    let preview = imaging::decode(&large, 100, 100).unwrap();
    assert!(preview.width <= 100);
    let full = out.join("full-resolution.png");
    imaging::export(&large, &full, &[]).unwrap();
    let decoded = imaging::decode(&full, 100, 100).unwrap();
    assert_eq!(
        (decoded.source_width, decoded.source_height),
        (6000, 4000),
        "Export used display resolution"
    );
    assert!(imaging::export(&source, &source, &[]).is_err());
    assert_eq!(fs::read(&source).unwrap(), original);
    assert!(imaging::export(
        &source,
        &out.join("invalid.png"),
        &[ImageEdit::Resize {
            width: 0,
            height: 5
        }]
    )
    .is_err());
    assert!(!out.join("invalid.png").exists());
    let heic = out.join("codec.heic");
    assert_eq!(
        imaging::export(&source, &heic, &[]).is_ok(),
        imaging::heic_encode_available()
    );
    assert_eq!(heic.exists(), imaging::heic_encode_available());
    let corrupt = out.join("corrupt.jpg");
    fs::write(&corrupt, b"not a jpeg").unwrap();
    assert!(imaging::decode(&corrupt, 10, 10).is_err());
    let small = fixtures.join("image-small.png");
    let unmarked = imaging::decode(&small, 640, 480).unwrap();
    use model::AnnotationKind::*;
    for (index, kind) in [
        Ink, Highlight, Underline, Strikeout, Note, Rectangle, Ellipse, Arrow, Text,
    ]
    .into_iter()
    .enumerate()
    {
        let marked_path = out.join(format!("markup-{index}.png"));
        imaging::export(
            &small,
            &marked_path,
            &[ImageEdit::Annotate {
                kind,
                points: vec![[0.1, 0.2], [0.6, 0.6]],
                text: "Image note".into(),
            }],
        )
        .unwrap();
        let marked = imaging::decode(&marked_path, 640, 480).unwrap();
        assert_eq!((marked.width, marked.height), (640, 480));
        assert!(
            marked
                .pixels
                .iter()
                .zip(&unmarked.pixels)
                .any(|(a, b)| a != b),
            "Annotation {kind:?} had no visible result"
        );
    }
    assert!(imaging::export(
        &small,
        &out.join("invisible-ink.png"),
        &[ImageEdit::Annotate {
            kind: Ink,
            points: vec![[0.1, 0.1]],
            text: String::new()
        }]
    )
    .is_err());
    assert!(imaging::export(
        &small,
        &out.join("truncated-text.png"),
        &[ImageEdit::Annotate {
            kind: Text,
            points: vec![[0.1, 0.1]],
            text: "x".repeat(4097)
        }]
    )
    .is_err());
    assert!(
        !fs::read_dir(&out).unwrap().any(|e| e
            .unwrap()
            .path()
            .extension()
            .is_some_and(|x| x == "tmp")),
        "Temporary output leaked"
    );
}
