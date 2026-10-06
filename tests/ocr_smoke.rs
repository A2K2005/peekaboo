#![allow(dead_code)]
#[path = "../src/imaging.rs"]
mod imaging;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/ocr.rs"]
mod ocr;

#[test]
#[ignore = "Requires an installed Windows OCR language and fixtures/ocr-text.png"]
fn recognizes_local_screenshot_text() {
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap() };
    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe { CoUninitialize() }
        }
    }
    let _com = Com;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/ocr-text.png");
    let start = std::time::Instant::now();
    let text = ocr::recognize(&path, &[]).unwrap();
    println!("OCR completed in {:?}: {text}", start.elapsed());
    assert!(text.contains("Preview for Windows"));
    assert!(text.contains("12345"));
}
