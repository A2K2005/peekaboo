#![allow(dead_code)]
#[path = "../src/background.rs"]
mod background;
#[path = "../src/imaging.rs"]
mod imaging;
#[path = "../src/model.rs"]
mod model;

#[test]
#[ignore = "Requires the optional local AI pack and a photo fixture"]
fn background_creates_transparent_full_resolution_png() {
    use windows::Win32::System::Com::*;
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap();
    }
    let source = std::path::PathBuf::from(
        std::env::var_os("PFW_BG_FIXTURE").expect("Set PFW_BG_FIXTURE to a photo"),
    );
    let out = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts/background");
    std::fs::create_dir_all(&out).unwrap();
    let output = out.join(format!("result-{}.png", std::process::id()));
    let original = imaging::decode(&source, u32::MAX, u32::MAX).unwrap();
    let report = background::remove(&source, &output, &[]).unwrap();
    println!("{report}");
    let result = imaging::decode(&output, u32::MAX, u32::MAX).unwrap();
    assert_eq!(
        (result.width, result.height),
        (original.width, original.height)
    );
    assert!(
        result.pixels.chunks_exact(4).any(|p| p[3] < 20),
        "No background removed"
    );
    assert!(
        result.pixels.chunks_exact(4).any(|p| p[3] > 235),
        "No foreground retained"
    );
    println!("Output: {}", output.display());
    unsafe {
        CoUninitialize();
    }
}
