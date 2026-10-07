#![allow(dead_code)]
//! Needs a Windows OCR language, such as English, which Windows installs with
//! the display language.
#[path = "../src/imaging.rs"]
mod imaging;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/ocr.rs"]
mod ocr;

use model::{AnnotationKind, Frame, ImageEdit};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};
static NEXT: AtomicUsize = AtomicUsize::new(0);

fn com() {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap() };
}
fn text(at: [f32; 2], words: &str) -> ImageEdit {
    ImageEdit::Annotate {
        kind: AnnotationKind::Text,
        points: vec![at],
        text: words.into(),
    }
}

/// White paper with two lines of text, drawn by the app's own markup code.
fn page() -> PathBuf {
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("artifacts/ocr")
        .join(format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    std::fs::create_dir_all(&out).unwrap();
    let paper = out.join("paper.png");
    let white = Frame {
        width: 1200,
        height: 400,
        pixels: vec![255; 1200 * 400 * 4],
        page_count: 1,
        source_width: 1200,
        source_height: 400,
    };
    imaging::export_frame(&white, &paper).unwrap();
    let page = out.join("page.png");
    // Lines are drawn out of order. "Right side" sits on the second row, close
    // after "Second line of text", but starts a little higher.
    let edits = [
        text([0.35, 0.53], "Right side"),
        text([0.05, 0.55], "Second line of text"),
        text([0.05, 0.1], "Preview for Windows 12345"),
    ];
    imaging::export(&paper, &page, &edits).unwrap();
    page
}

#[test]
fn text_layer_has_lines_in_order_and_a_box_per_char() {
    com();
    let page = page();
    let start = Instant::now();
    let layer = ocr::recognize_layer(&page, &[]).unwrap();
    println!("OCR in {:?}: {:?}", start.elapsed(), layer.text);
    assert_eq!(layer.boxes.len(), layer.text.chars().count());
    let lines: Vec<&str> = layer.text.lines().collect();
    assert!(lines.len() >= 2, "{:?}", layer.text);
    let second = layer.text.find("Second line").unwrap();
    assert!(
        layer.text.find("Right side").unwrap() > second,
        "Row read right to left"
    );
    assert!(lines[0].contains("Preview for Windows"), "{:?}", lines[0]);
    assert!(lines[0].contains("12345"));
    assert!(lines[1].contains("Second line"), "{:?}", lines[1]);
    assert!(layer
        .boxes
        .iter()
        .flatten()
        .all(|v| (0.0..=1.0).contains(v)));
    let chars: Vec<char> = layer.text.chars().collect();
    let first = layer.text.find("Preview").unwrap();
    let windows = layer.text[..layer.text.find("Windows").unwrap()]
        .chars()
        .count();
    let first = layer.text[..first].chars().count();
    let (p, w) = (layer.boxes[first], layer.boxes[windows]);
    // Text starts 5% from the left; the first line sits in the top half.
    assert!((p[0] - 0.05).abs() < 0.03, "{p:?}");
    assert!(p[1] > 0.05 && p[3] < 0.5, "{p:?}");
    assert!(w[0] > p[2], "Words are out of order: {p:?} {w:?}");
    let second = chars.iter().position(|&c| c == '\n').unwrap() + 1;
    assert!(
        layer.boxes[second][1] > p[3],
        "The second line is not below the first"
    );
    for (c, b) in chars.iter().zip(&layer.boxes) {
        if c.is_whitespace() {
            assert_eq!(b[0], b[2], "A separator has width");
        } else {
            assert!(b[2] > b[0] && b[3] > b[1], "{c} has no box");
        }
    }
    assert!(ocr::recognize(&page, &[]).unwrap().contains("12345"));
    // Boxes are relative to the edited image: the text starts at 0.05 / 0.6 of this crop.
    let crop = [ImageEdit::Crop {
        left: 0.0,
        top: 0.0,
        right: 0.6,
        bottom: 0.5,
    }];
    let cropped = ocr::recognize_layer(&page, &crop).unwrap();
    assert!(cropped.text.starts_with("Preview"), "{:?}", cropped.text);
    assert!(
        (cropped.boxes[0][0] - 0.05 / 0.6).abs() < 0.03,
        "{:?}",
        cropped.boxes[0]
    );
}

/// Two columns of three lines each: the left column reads first.
#[test]
fn two_columns_keep_column_order() {
    com();
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("artifacts/ocr")
        .join(format!("columns-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let paper = out.join("paper.png");
    let white = Frame {
        width: 1600,
        height: 600,
        pixels: vec![255; 1600 * 600 * 4],
        page_count: 1,
        source_width: 1600,
        source_height: 600,
    };
    imaging::export_frame(&white, &paper).unwrap();
    let left = ["Alpha bravo charlie", "Delta echo foxtrot", "Golf hotel india"];
    let right = ["Kilo lima mike", "November oscar papa", "Quebec romeo sierra"];
    let mut edits = Vec::new();
    for (i, (l, r)) in left.iter().zip(right).enumerate() {
        let y = 0.1 + 0.25 * i as f32;
        edits.push(text([0.04, y], l));
        edits.push(text([0.54, y], r));
    }
    let page = out.join("columns.png");
    imaging::export(&paper, &page, &edits).unwrap();
    let layer = ocr::recognize_layer(&page, &[]).unwrap();
    println!("{:?}", layer.text);
    let at = |word: &str| layer.text.find(word).unwrap_or_else(|| panic!("{word} missing: {:?}", layer.text));
    assert!(at("Alpha") < at("Delta") && at("Delta") < at("Golf"));
    assert!(at("Golf") < at("Kilo"), "Columns interleaved: {:?}", layer.text);
    assert!(at("Kilo") < at("November") && at("November") < at("Quebec"));
}

#[test]
fn blank_image_gives_empty_layer() {
    com();
    let page = page();
    let blank = [ImageEdit::Crop {
        left: 0.0,
        top: 0.9,
        right: 1.0,
        bottom: 1.0,
    }];
    let layer = ocr::recognize_layer(&page, &blank).unwrap();
    assert_eq!(layer, model::TextLayer::default());
    assert!(ocr::recognize(&page, &blank).is_err());
}

/// Speed on the text page and on PFW_OCR_PHOTO, a photo with text.
#[test]
#[ignore = "Timing run; set PFW_OCR_PHOTO"]
fn ocr_speed() {
    com();
    let mut inputs = vec![page()];
    if let Some(photo) = std::env::var_os("PFW_OCR_PHOTO") {
        inputs.push(PathBuf::from(photo));
    }
    for input in inputs {
        ocr::recognize_layer(&input, &[]).unwrap();
        let mut times = Vec::new();
        let mut text = String::new();
        for _ in 0..5 {
            let start = Instant::now();
            text = ocr::recognize_layer(&input, &[]).unwrap().text;
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        println!("{}: {times:.0?} ms\n{text}", input.display());
    }
}
