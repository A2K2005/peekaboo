#![allow(dead_code)]
#[path = "../src/imaging.rs"]
mod imaging;
#[path = "../src/model.rs"]
mod model;

use model::{BatchJob, BatchResize, ExportOptions, ImageFormat};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    sync::Mutex,
};

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
fn out_dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("artifacts/batch")
        .join(format!("{}-{name}", std::process::id()))
}

#[test]
fn batch_rotates_resizes_and_never_overwrites() {
    com();
    let out = out_dir("rotate");
    std::fs::create_dir_all(&out).unwrap();
    let existing = out.join("image-small.png");
    std::fs::write(&existing, b"keep me").unwrap();
    let corrupt = out.join("corrupt.jpg");
    std::fs::write(&corrupt, b"not a jpeg").unwrap();
    let small = fixture("image-small.png");
    let large = fixture("image-24mp.jpg");
    let inputs = vec![small.clone(), large.clone(), small.clone(), corrupt.clone()];
    let calls = Mutex::new(Vec::new());
    let job = BatchJob {
        quarter_turns: 1,
        resize: Some(BatchResize::Percent(50.0)),
        options: None,
    };
    let results = imaging::batch(
        &inputs,
        &out,
        &job,
        &|done, total| calls.lock().unwrap().push((done, total)),
        &AtomicBool::new(false),
    );
    let mut calls = calls.into_inner().unwrap();
    calls.sort();
    assert_eq!(calls, vec![(1, 4), (2, 4), (3, 4), (4, 4)]);
    assert_eq!(results.len(), 4);
    assert_eq!(results[0].input, small);
    assert_eq!(results[0].output, Ok(out.join("image-small (2).png")));
    assert_eq!(results[1].output, Ok(out.join("image-24mp.jpg")));
    assert_eq!(results[2].output, Ok(out.join("image-small (3).png")));
    assert!(results[3].output.is_err(), "A corrupt file did not fail");
    assert_eq!(std::fs::read(&existing).unwrap(), b"keep me");
    assert_eq!(std::fs::read(&corrupt).unwrap(), b"not a jpeg");
    // 6000 x 4000 turned right and halved.
    let frame = imaging::decode(&out.join("image-24mp.jpg"), 100, 100).unwrap();
    assert_eq!((frame.source_width, frame.source_height), (2000, 3000));
    let frame = imaging::decode(&out.join("image-small (2).png"), 1000, 1000).unwrap();
    assert_eq!((frame.width, frame.height), (240, 320));
}

#[test]
fn batch_converts_with_size_limits() {
    com();
    let out = out_dir("convert").join("new folder");
    let inputs = vec![fixture("image-24mp.jpg"), fixture("image-small.png")];
    let job = BatchJob {
        quarter_turns: 0,
        resize: Some(BatchResize::MaxEdge(1000)),
        options: Some(ExportOptions {
            format: ImageFormat::WebP,
            quality: 0.8,
            lossless: false,
        }),
    };
    let results = imaging::batch(&inputs, &out, &job, &|_, _| {}, &AtomicBool::new(false));
    let sizes: Vec<_> = results
        .iter()
        .map(|r| {
            let path = r.output.clone().unwrap();
            assert_eq!(path.extension().unwrap(), "webp");
            let frame = imaging::decode(&path, 2000, 2000).unwrap();
            (frame.width, frame.height)
        })
        .collect();
    assert_eq!(sizes, vec![(1000, 667), (640, 480)]);
    let exact = BatchJob {
        resize: Some(BatchResize::Pixels {
            width: 30,
            height: 10,
        }),
        options: None,
        ..job
    };
    let results = imaging::batch(
        &inputs[1..],
        &out,
        &exact,
        &|_, _| {},
        &AtomicBool::new(false),
    );
    let path = results[0].output.clone().unwrap();
    let frame = imaging::decode(&path, 100, 100).unwrap();
    assert_eq!((frame.width, frame.height), (30, 10));
}

#[test]
fn batch_cancel_skips_files_not_started() {
    com();
    let out = out_dir("cancel");
    let inputs = vec![fixture("image-small.png"); 12];
    let job = BatchJob {
        quarter_turns: 2,
        resize: None,
        options: None,
    };
    let cancel = AtomicBool::new(true);
    let results = imaging::batch(&inputs, &out, &job, &|_, _| {}, &cancel);
    assert!(results.iter().all(|r| r.output == Err("Canceled.".into())));
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0);

    let cancel = AtomicBool::new(false);
    let started = AtomicUsize::new(0);
    let results = imaging::batch(
        &inputs,
        &out,
        &job,
        &|_, _| {
            started.fetch_add(1, Ordering::Relaxed);
            cancel.store(true, Ordering::Relaxed);
        },
        &cancel,
    );
    let saved = results.iter().filter(|r| r.output.is_ok()).count();
    assert!((1..=4).contains(&saved), "{saved} files saved after cancel");
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), saved);
    assert_eq!(started.load(Ordering::Relaxed), 12);
}
