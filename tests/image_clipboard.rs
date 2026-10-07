#![allow(dead_code)]
//! Clipboard data formats. These tests never touch the real clipboard.
#[path = "../src/imaging.rs"]
mod imaging;
#[path = "../src/model.rs"]
mod model;

use model::Frame;
use std::path::PathBuf;

fn com() {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap() };
}
fn fixture(name: &str) -> PathBuf {
    std::env::var_os("PFW_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures"))
        .join(name)
}
/// The small fixture with its left half at 50% opacity.
fn photo_with_alpha() -> Frame {
    let mut frame = imaging::decode(&fixture("image-small.png"), 640, 480).unwrap();
    for (i, p) in frame.pixels.chunks_exact_mut(4).enumerate() {
        if i % 640 < 320 {
            for c in p.iter_mut() {
                *c = ((*c as u32 * 128 + 127) / 255) as u8;
            }
        }
    }
    frame
}
fn max_difference(a: &Frame, b: &Frame) -> u8 {
    assert_eq!((a.width, a.height), (b.width, b.height));
    a.pixels
        .iter()
        .zip(&b.pixels)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap()
}
/// A packed DIB with a BITMAPINFOHEADER, bottom-up rows padded to 4 bytes.
fn dib(frame: &Frame, bits: u16, alpha: Option<u8>) -> Vec<u8> {
    let bytes = (bits / 8) as usize;
    let stride = (frame.width as usize * bytes + 3) & !3;
    let mut out = Vec::new();
    for v in [40u32, frame.width, frame.height] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    for v in [0u32, (stride * frame.height as usize) as u32, 0, 0, 0, 0] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for row in frame.pixels.chunks_exact(frame.width as usize * 4).rev() {
        let start = out.len();
        for p in row.chunks_exact(4) {
            out.extend_from_slice(&p[..3]);
            if let Some(a) = alpha {
                out.push(a);
            }
        }
        out.resize(start + stride, 0);
    }
    out
}

#[test]
fn clipboard_formats_round_trip_alpha() {
    com();
    let frame = photo_with_alpha();
    let (dibv5, png) = imaging::encode_clipboard(&frame).unwrap();
    assert_eq!(u32::from_le_bytes(dibv5[0..4].try_into().unwrap()), 124);
    assert_eq!(dibv5.len(), 124 + 640 * 480 * 4);
    // Bottom-up rows with straight alpha: the first stored pixel is the
    // bottom-left one, at 50% opacity.
    let bottom_left = &frame.pixels[479 * 640 * 4..479 * 640 * 4 + 4];
    assert_eq!(dibv5[124 + 3], 128);
    assert!(dibv5[124 + 2].abs_diff(((bottom_left[2] as u32 * 255 + 64) / 128) as u8) <= 2);
    assert_eq!(&png[1..4], b"PNG");
    for (bytes, is_dib) in [(&png, false), (&dibv5, true)] {
        let decoded = imaging::decode_clipboard(bytes, is_dib).unwrap();
        assert!(max_difference(&decoded, &frame) <= 1, "dib={is_dib}");
    }
}

#[test]
fn plain_dibs_decode_with_padding_and_zero_alpha() {
    com();
    let photo = imaging::decode(&fixture("image-small.png"), 640, 480).unwrap();
    // 7 pixels wide makes 24-bit rows need padding.
    let crop = imaging::decode_edited(
        &fixture("image-small.png"),
        7,
        3,
        &[model::ImageEdit::Crop {
            left: 0.45,
            top: 0.45,
            right: 0.45 + 7.0 / 640.0,
            bottom: 0.45 + 3.0 / 480.0,
        }],
    )
    .unwrap();
    assert_eq!((crop.width, crop.height), (7, 3));
    let decoded = imaging::decode_clipboard(&dib(&crop, 24, None), true).unwrap();
    assert_eq!(decoded.pixels, crop.pixels);
    // 32-bit rows whose alpha bytes are all 0 are opaque, not invisible.
    let decoded = imaging::decode_clipboard(&dib(&photo, 32, Some(0)), true).unwrap();
    assert_eq!(decoded.pixels, photo.pixels);
    // A CF_DIBV5 with an alpha mask but no alpha values is opaque too.
    let (mut dibv5, _) = imaging::encode_clipboard(&photo).unwrap();
    for p in dibv5[124..].chunks_exact_mut(4) {
        p[3] = 0;
    }
    let decoded = imaging::decode_clipboard(&dibv5, true).unwrap();
    assert_eq!(decoded.pixels, photo.pixels);
    for damaged in [
        vec![5, 0, 0, 0],
        dib(&photo, 24, None)[..30].to_vec(),
        vec![],
    ] {
        assert!(imaging::decode_clipboard(&damaged, true).is_err());
    }
    assert!(imaging::decode_clipboard(b"not an image", false).is_err());
}

#[test]
fn clipboard_image_saves_as_png() {
    com();
    let frame = photo_with_alpha();
    let (_, png) = imaging::encode_clipboard(&frame).unwrap();
    let pasted = imaging::decode_clipboard(&png, false).unwrap();
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("artifacts/clipboard")
        .join(format!("{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let path = out.join("new-from-clipboard.png");
    imaging::export_frame(&pasted, &path).unwrap();
    let saved = imaging::decode(&path, 640, 480).unwrap();
    assert!(max_difference(&saved, &frame) <= 1);
}
