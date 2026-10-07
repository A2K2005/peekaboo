// PDF engine checks (W1-B). Run serially: PDFium is process-global.
// cargo test --release --test pdf_engine -- --test-threads=1 --nocapture
#![allow(dead_code)]
#[path = "../src/imaging.rs"]
mod imaging;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/pdf.rs"]
mod pdf;
mod pdf_support;

use model::{AnnotationKind, PdfEdit};
use pdf::PdfEngine;
use pdf_support::*;
use std::{sync::atomic::AtomicBool, time::Instant};

#[test]
fn page_sizes_follow_edits_rotation_and_crop() {
    let dir = out_dir("sizes");
    let path = dir.join("sizes.pdf");
    pages_pdf(
        &path,
        &[
            ("[0 0 612 792]", text_at(50.0, 700.0, 12.0, "Letter")),
            ("[0 0 842 595]", text_at(50.0, 500.0, 12.0, "A4 landscape")),
            ("[10 20 210 120]", text_at(20.0, 50.0, 12.0, "Small")),
        ],
    );
    let mut engine = PdfEngine::new().unwrap();
    assert_eq!(
        engine.page_sizes(&path, &[]).unwrap(),
        vec![[612.0, 792.0], [842.0, 595.0], [200.0, 100.0]]
    );
    let edits = [
        PdfEdit::RotateRight { page: 0 },
        PdfEdit::Crop {
            page: 1,
            left: 0.0,
            top: 0.0,
            right: 0.5,
            bottom: 1.0,
        },
        PdfEdit::Delete { page: 2 },
        PdfEdit::InsertBlank { at: 2 },
    ];
    assert_eq!(
        engine.page_sizes(&path, &edits).unwrap(),
        vec![[792.0, 612.0], [421.0, 595.0], [421.0, 595.0]]
    );
    let frame = engine.render_edited(&path, 0, 792, 612, &edits).unwrap();
    assert_eq!((frame.source_width, frame.source_height), (792, 612));
}

#[test]
fn region_tiles_match_the_full_page_and_draw_fields_and_markup() {
    let dir = out_dir("region");
    let form = dir.join("form.pdf");
    acroform_pdf(&form);
    let edits = [PdfEdit::Annotate {
        page: 0,
        kind: AnnotationKind::Rectangle,
        points: vec![[0.1, 0.5], [0.5, 0.7]],
        text: String::new(),
    }];
    let mut engine = PdfEngine::new().unwrap();
    let (width, height) = (918, 1188); // 1.5 device pixels per point
    let full = engine
        .render_region(&form, 0, 1.5, [0, 0, width, height], &edits)
        .unwrap();
    assert_eq!(
        (full.width, full.height, full.page_count),
        (width, height, 1)
    );
    assert_eq!((full.source_width, full.source_height), (612, 792));
    let reference = engine
        .render_edited(&form, 0, width, height, &edits)
        .unwrap();
    assert_eq!(changed(&full.pixels, &reference.pixels), 0);
    // Stitch 256-pixel tiles and compare with the full render.
    let mut stitched = vec![0u8; full.pixels.len()];
    for ty in (0..height).step_by(256) {
        for tx in (0..width).step_by(256) {
            let tile = engine
                .render_region(&form, 0, 1.5, [tx, ty, 256, 256], &edits)
                .unwrap();
            for y in 0..256.min(height - ty) {
                let source = (y * 256 * 4) as usize;
                let target = (((ty + y) * width + tx) * 4) as usize;
                let n = (256.min(width - tx) * 4) as usize;
                stitched[target..target + n].copy_from_slice(&tile.pixels[source..source + n]);
            }
        }
    }
    let differing = changed(&stitched, &full.pixels);
    assert!(
        differing <= (width * height / 1000) as usize,
        "{differing} pixels differ"
    );
    // The text field value (form layer) and the rectangle (annotation) both draw.
    let field = [50.0 / 612.0, 92.0 / 792.0, 350.0 / 612.0, 142.0 / 792.0];
    assert!(dark_in(&full.pixels, width, height, field) > 100);
    let edge = [0.09, 0.49, 0.11, 0.71];
    assert!(dark_in(&full.pixels, width, height, edge) > 50);
    assert!(engine
        .render_region(&form, 0, 0.0, [0, 0, 10, 10], &edits)
        .is_err());
    assert!(engine
        .render_region(&form, 0, 1.0, [0, 0, 0, 10], &edits)
        .is_err());
    assert!(engine
        .render_region(&form, 1, 1.0, [0, 0, 10, 10], &edits)
        .is_err());
}

#[test]
fn render_timings_on_500_pages() {
    let source = fixture("500-pages-50mb.pdf");
    let mut engine = PdfEngine::new().unwrap();
    let start = Instant::now();
    let first = engine
        .render_region(&source, 0, 1.0, [0, 0, 612, 792], &[])
        .unwrap();
    let first_ms = start.elapsed().as_secs_f64() * 1000.0;
    assert!(dark_in(&first.pixels, 612, 792, [0.05, 0.08, 0.6, 0.14]) > 100);
    let mut tiles = Vec::new();
    for page in 0..20 {
        let start = Instant::now();
        let tile = engine
            .render_region(&source, page, 2.0, [0, 0, 512, 512], &[])
            .unwrap();
        tiles.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!((tile.width, tile.height), (512, 512));
    }
    let mut thumbnails = Vec::new();
    for page in 0..100 {
        let start = Instant::now();
        engine
            .render_region(&source, page, 0.2, [0, 0, 122, 158], &[])
            .unwrap();
        thumbnails.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    println!(
        "render 500-page PDF: open + first page {first_ms:.1} ms; 512x512 tile at 2x median {:.2} ms (n=20); 122x158 thumbnail median {:.2} ms (n=100)",
        median(tiles),
        median(thumbnails)
    );
}

#[test]
fn text_layer_puts_columns_in_reading_order() {
    let dir = out_dir("text");
    let path = dir.join("columns.pdf");
    // Right column first and bottom to top, then the left column; heading and
    // footer last. A content-order reader gets all of this backwards.
    let mut columns = String::new();
    for i in (0..3).rev() {
        columns += &text_at(
            320.0,
            600.0 - i as f32 * 14.0,
            12.0,
            &format!("Right {}", i + 1),
        );
    }
    for i in (0..3).rev() {
        columns += &text_at(
            50.0,
            600.0 - i as f32 * 14.0,
            12.0,
            &format!("Left {}", i + 1),
        );
    }
    let footer = "Footer text that runs across the whole width of the page, both columns";
    let heading = "A heading that spans the two columns";
    columns += &text_at(50.0, 300.0, 12.0, footer);
    columns += &text_at(50.0, 680.0, 18.0, heading);
    let single = [
        "The first line of a normal paragraph of text.",
        "The second line follows directly underneath it.",
        "A third line ends the paragraph.",
        "Another paragraph starts after a gap.",
    ];
    let mut plain = String::new();
    for (i, line) in single.iter().enumerate() {
        let gap = if i == 3 { 18.0 } else { 0.0 };
        plain += &text_at(72.0, 700.0 - i as f32 * 14.0 - gap, 12.0, line);
    }
    text_pdf(&path, &[columns, plain]);
    let mut engine = PdfEngine::new().unwrap();
    let layer = engine.text_layer(&path, 0, &[]).unwrap();
    assert_eq!(
        layer.text,
        format!("{heading}\nLeft 1\nLeft 2\nLeft 3\nRight 1\nRight 2\nRight 3\n{footer}")
    );
    assert_eq!(layer.boxes.len(), layer.text.chars().count());
    assert!(layer
        .boxes
        .iter()
        .flatten()
        .all(|v| (0.0..=1.0).contains(v)));
    // Normal single-column text comes out as PDFium orders it.
    let single_layer = engine.text_layer(&path, 1, &[]).unwrap();
    let pdfium = engine.page_text(&path, 1).unwrap().replace("\r\n", "\n");
    assert_eq!(single_layer.text, pdfium.trim());
    assert_eq!(single_layer.text, single.join("\n"));
    // Boxes land on the rendered glyphs, also on a rotated page.
    for (edits, size) in [
        (vec![], (612, 792)),
        (vec![PdfEdit::RotateRight { page: 0 }], (792, 612)),
    ] {
        let layer = engine.text_layer(&path, 0, &edits).unwrap();
        let start = layer.text.find("Left 1").unwrap();
        let union = layer.boxes[start..start + 6]
            .iter()
            .fold([1.0f32, 1.0, 0.0, 0.0], |u, b| {
                [
                    u[0].min(b[0]),
                    u[1].min(b[1]),
                    u[2].max(b[2]),
                    u[3].max(b[3]),
                ]
            });
        let frame = engine
            .render_edited(&path, 0, size.0, size.1, &edits)
            .unwrap();
        let inside = dark_in(&frame.pixels, frame.width, frame.height, union);
        assert!(inside > 30, "{edits:?}: {inside} dark pixels in {union:?}");
        if edits.is_empty() {
            assert!((union[0] - 50.0 / 612.0).abs() < 0.01, "{union:?}");
        } else {
            // Rotated clockwise: the left margin becomes the top margin.
            assert!((union[1] - 50.0 / 612.0).abs() < 0.01, "{union:?}");
        }
    }
}

#[test]
fn search_finds_every_hit_in_500_pages_quickly() {
    let source = fixture("500-pages-50mb.pdf");
    let mut engine = PdfEngine::new().unwrap();
    let cancel = AtomicBool::new(false);
    let start = Instant::now();
    let hits = engine
        .search(&source, "synthetic page", false, &cancel, &[])
        .unwrap();
    let all_ms = start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(hits.len(), 500);
    assert!(hits
        .iter()
        .enumerate()
        .all(|(i, h)| h.page == i as u32 && h.rects.len() == 1));
    let rect = hits[249].rects[0];
    assert!(
        (rect[0] - 50.0 / 612.0).abs() < 0.01 && rect[1] < 0.14 && rect[3] > 0.1,
        "{rect:?}"
    );
    let start = Instant::now();
    let one = engine
        .search(&source, "Synthetic page 250 of 500", true, &cancel, &[])
        .unwrap();
    let one_ms = start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].page, 249);
    assert!(engine
        .search(&source, "synthetic page", true, &cancel, &[])
        .unwrap()
        .is_empty());
    let deleted = engine
        .search(
            &source,
            "Synthetic page 250 of",
            true,
            &cancel,
            &[PdfEdit::Delete { page: 0 }],
        )
        .unwrap();
    assert_eq!(deleted[0].page, 248);
    let fresh = std::time::Instant::now();
    drop(engine);
    let mut engine = PdfEngine::new().unwrap();
    let cold = engine
        .search(&source, "Synthetic page 500 of 500", true, &cancel, &[])
        .unwrap();
    let cold_ms = fresh.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(cold.len(), 1);
    println!(
        "search 500-page PDF: 500 hits {all_ms:.0} ms; 1 hit {one_ms:.0} ms; new engine, open and search {cold_ms:.0} ms"
    );
    assert!(all_ms < 1000.0 && one_ms < 1000.0 && cold_ms < 1000.0);
    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        engine
            .search(&source, "page", false, &cancel, &[])
            .unwrap_err(),
        pdf::SEARCH_CANCELED
    );
    assert!(engine
        .search(&source, "", false, &AtomicBool::new(false), &[])
        .is_err());
}
