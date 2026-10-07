#![allow(dead_code)]
//! Needs the AI pack in runtime/ai (see tools/fetch-model.ps1).
#[path = "../src/background.rs"]
mod background;
#[path = "../src/imaging.rs"]
mod imaging;
#[path = "../src/model.rs"]
mod model;

use std::{path::PathBuf, time::Instant};

fn com() {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap() };
}
fn out_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("artifacts/background")
        .join(format!("{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn cutout_keeps_size_and_alpha_rules() {
    com();
    let photo = std::env::var_os("PFW_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures"))
        .join("image-small.png");
    let original = imaging::decode(&photo, 640, 480).unwrap();
    let edits = [model::ImageEdit::RotateRight];
    let result = background::cutout(&photo, &edits).unwrap();
    assert_eq!(
        (result.width, result.height),
        (480, 640),
        "Edits were not applied"
    );
    let rotated = imaging::decode_edited(&photo, 480, 640, &edits).unwrap();
    for (cut, source) in result
        .pixels
        .chunks_exact(4)
        .zip(rotated.pixels.chunks_exact(4))
    {
        // Premultiplied: no channel above alpha, and alpha scales the source.
        assert!(cut[..3].iter().all(|&c| c <= cut[3]));
        let a = cut[3] as f32 / 255.0;
        assert!((cut[0] as f32 - source[0] as f32 * a).abs() <= 1.0);
    }
    let out = out_dir();
    let saved = out.join("cutout.png");
    background::remove(&photo, &saved, &[]).unwrap();
    let decoded = imaging::decode(&saved, 640, 480).unwrap();
    assert_eq!(
        (decoded.width, decoded.height),
        (original.width, original.height)
    );
    assert!(
        background::remove(&photo, &saved, &[]).is_err(),
        "Overwrote a file"
    );
    assert!(background::remove(&photo, &out.join("cutout.jpg"), &[]).is_err());
}

/// Every photo in PFW_PHOTOS: time, and both transparent and opaque areas.
/// Masks go to artifacts/background/<pid>/ for comparison with the reference.
#[test]
#[ignore = "Needs PFW_PHOTOS, a folder of 12 MP photos"]
fn cutout_photos() {
    com();
    let folder = PathBuf::from(std::env::var_os("PFW_PHOTOS").expect("Set PFW_PHOTOS"));
    let mut photos: Vec<PathBuf> = std::fs::read_dir(folder)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            name.ends_with(".jpg") && name.starts_with(['0', '1'])
        })
        .collect();
    photos.sort();
    let out = out_dir();
    let start = Instant::now();
    background::cutout(&photos[0], &[]).unwrap();
    println!(
        "first use, with model load: {:.2} s",
        start.elapsed().as_secs_f32()
    );
    let mut times = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        background::cutout(&photos[0], &[]).unwrap();
        times.push(start.elapsed().as_secs_f32());
    }
    println!("cutout 12 MP, 5 runs: {times:.2?}");
    for photo in &photos {
        let start = Instant::now();
        let result = background::cutout(photo, &[]).unwrap();
        let seconds = start.elapsed().as_secs_f32();
        let clear = result.pixels.chunks_exact(4).filter(|p| p[3] < 20).count();
        let solid = result.pixels.chunks_exact(4).filter(|p| p[3] > 235).count();
        let total = (result.width * result.height) as f32;
        println!(
            "{}: {seconds:.2} s, {:.0}% clear, {:.0}% solid",
            photo.file_name().unwrap().to_string_lossy(),
            clear as f32 / total * 100.0,
            solid as f32 / total * 100.0
        );
        let mask = model::Frame {
            pixels: result
                .pixels
                .chunks_exact(4)
                .flat_map(|p| [p[3], p[3], p[3], 255])
                .collect(),
            ..result
        };
        let small = imaging::resize_frame(&mask, 1000, 750).unwrap();
        let name = photo.with_extension("png");
        imaging::export_frame(&small, &out.join(name.file_name().unwrap())).unwrap();
    }
    let start = Instant::now();
    let saved = out.join("save-timing.png");
    background::remove(&photos[0], &saved, &[]).unwrap();
    println!(
        "cutout plus PNG save: {:.2} s",
        start.elapsed().as_secs_f32()
    );
}
