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

#[test]
fn outline_and_metadata_follow_edits() {
    let dir = out_dir("outline");
    let path = dir.join("outline.pdf");
    let page = "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>";
    let mut objects: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Outlines 6 0 R >>".into(),
        "<< /Type /Pages /Count 3 /Kids [3 0 R 4 0 R 5 0 R] >>".into(),
        page.into(),
        page.into(),
        page.into(),
        "<< /Type /Outlines /First 7 0 R /Last 9 0 R /Count 3 >>".into(),
        "<< /Title (Chapter 1) /Parent 6 0 R /Next 9 0 R /First 8 0 R /Last 8 0 R /Count 1 /Dest [3 0 R /Fit] >>".into(),
        "<< /Title (Section 1.1) /Parent 7 0 R /A << /S /GoTo /D [4 0 R /XYZ 0 792 0] >> >>".into(),
        // "Chapter 2" as a UTF-16BE string.
        "<< /Title <FEFF0043006800610070007400650072002000320021> /Parent 6 0 R /Prev 7 0 R /Dest [5 0 R /Fit] >>".into(),
        "<< /Title (Test title) /Author (Ada) /Subject (Engines) /Keywords (pdf, test) /Creator (Writer) /Producer (Generator) /CreationDate (D:20261007120000Z) /ModDate (D:20261007130000Z) >>".into(),
    ];
    write_pdf(&path, "1.7", &objects, "/Info 10 0 R");
    let mut engine = PdfEngine::new().unwrap();
    let outline = engine.outline(&path, &[]).unwrap();
    let flat: Vec<(&str, Option<u32>, u32)> = outline
        .iter()
        .map(|o| (o.title.as_str(), o.page, o.level))
        .collect();
    assert_eq!(
        flat,
        [
            ("Chapter 1", Some(0), 0),
            ("Section 1.1", Some(1), 1),
            ("Chapter 2!", Some(2), 0)
        ]
    );
    let deleted = engine
        .outline(&path, &[PdfEdit::Delete { page: 1 }])
        .unwrap();
    assert_eq!(deleted[1].page, None);
    assert_eq!(deleted[2].page, Some(1));
    let info = engine
        .metadata(&path, &[PdfEdit::Delete { page: 1 }])
        .unwrap();
    assert_eq!(
        (
            info.title.as_str(),
            info.author.as_str(),
            info.subject.as_str(),
            info.keywords.as_str(),
            info.creator.as_str(),
            info.producer.as_str(),
        ),
        (
            "Test title",
            "Ada",
            "Engines",
            "pdf, test",
            "Writer",
            "Generator"
        )
    );
    assert_eq!(info.created, "D:20261007120000Z");
    assert_eq!(info.modified, "D:20261007130000Z");
    assert_eq!(info.version, "1.7");
    assert_eq!(info.page_count, 2);
    assert!(!info.encrypted);
    assert_eq!(info.form, model::PdfFormType::None);
    // A damaged outline whose last entry points back to the first must end.
    objects[8] = "<< /Title (Loop) /Parent 6 0 R /Prev 7 0 R /Next 7 0 R >>".into();
    let looped = dir.join("loop.pdf");
    write_pdf(&looped, "1.7", &objects, "");
    assert_eq!(engine.outline(&looped, &[]).unwrap().len(), 3);
    assert!(engine.metadata(&looped, &[]).unwrap().title.is_empty());
    // Encryption is reported when qpdf can make an encrypted copy.
    let encrypted = dir.join("encrypted.pdf");
    let qpdf = std::env::var("QPDF").unwrap_or_else(|_| "qpdf".into());
    let made = std::process::Command::new(qpdf)
        .args(["--encrypt", "user", "owner", "256", "--"])
        .arg(&path)
        .arg(&encrypted)
        .status()
        .is_ok_and(|s| s.success());
    if made {
        engine.set_password(&encrypted, "user".into()).unwrap();
        assert!(engine.metadata(&encrypted, &[]).unwrap().encrypted);
    } else {
        eprintln!("qpdf not found: skipped the encrypted metadata check");
    }
}

#[test]
fn annotations_list_delete_and_edit_text() {
    let dir = out_dir("annotations");
    let path = dir.join("plain.pdf");
    text_pdf(
        &path,
        &[text_at(72.0, 700.0, 12.0, "Some text to mark up.")],
    );
    let mark = |kind, points: Vec<[f32; 2]>, text: &str| PdfEdit::Annotate {
        page: 0,
        kind,
        points,
        text: text.into(),
    };
    let mut edits = vec![
        mark(
            AnnotationKind::Highlight,
            vec![[0.1, 0.1], [0.4, 0.13]],
            "Highlight note",
        ),
        mark(AnnotationKind::Note, vec![[0.6, 0.2]], "Sticky"),
        mark(
            AnnotationKind::Text,
            vec![[0.1, 0.5], [0.7, 0.56]],
            "Free words",
        ),
    ];
    let mut engine = PdfEngine::new().unwrap();
    let list = engine.annotations(&path, 0, &edits).unwrap();
    let kinds: Vec<&str> = list.iter().map(|a| a.kind.as_str()).collect();
    assert_eq!(kinds, ["Highlight", "Text", "FreeText"]);
    let contents: Vec<&str> = list.iter().map(|a| a.contents.as_str()).collect();
    assert_eq!(contents, ["Highlight note", "Sticky", "Free words"]);
    let r = list[0].rect;
    assert!(
        (r[0] - 0.1).abs() < 0.01 && (r[1] - 0.1).abs() < 0.01,
        "{r:?}"
    );
    assert!(
        (r[2] - 0.4).abs() < 0.01 && (r[3] - 0.13).abs() < 0.01,
        "{r:?}"
    );
    let free = list[2].rect;
    let before = engine.render_edited(&path, 0, 612, 792, &edits).unwrap();
    edits.push(PdfEdit::SetAnnotationText {
        page: 0,
        index: 2,
        text: "Other words entirely".into(),
    });
    edits.push(PdfEdit::DeleteAnnotation { page: 0, index: 0 });
    let list = engine.annotations(&path, 0, &edits).unwrap();
    let contents: Vec<&str> = list.iter().map(|a| a.contents.as_str()).collect();
    assert_eq!(contents, ["Sticky", "Other words entirely"]);
    // The free text redraws with its new words.
    let after = engine.render_edited(&path, 0, 612, 792, &edits).unwrap();
    let area = |f: &model::Frame| -> Vec<u8> {
        let mut out = Vec::new();
        for y in (free[1] * 792.0) as usize..(free[3] * 792.0) as usize {
            let row = y * 612 * 4;
            out.extend(
                &f.pixels
                    [row + (free[0] * 612.0) as usize * 4..row + (free[2] * 612.0) as usize * 4],
            );
        }
        out
    };
    // Free text shows its words, not a box filled in the text color.
    let dark = dark_in(&before.pixels, 612, 792, free);
    assert!(
        dark > 100 && dark < 4000,
        "{dark} dark pixels in the free text box"
    );
    assert!(changed(&area(&before), &area(&after)) > 20);
    let saved = dir.join("saved.pdf");
    engine.save_copy(&path, &saved, &edits).unwrap();
    assert_eq!(engine.annotations(&saved, 0, &[]).unwrap(), list);
    // Form widgets are not deletable as annotations.
    let form = dir.join("form.pdf");
    acroform_pdf(&form);
    assert_eq!(
        engine
            .annotations(&form, 0, &[PdfEdit::DeleteAnnotation { page: 0, index: 0 }])
            .unwrap_err(),
        "Form fields cannot be deleted."
    );
    // Deleting a note also deletes its popup.
    let noted = dir.join("noted.pdf");
    let objects: Vec<String> = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Count 1 /Kids [3 0 R] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R 5 0 R] >>",
        "<< /Type /Annot /Subtype /Text /Rect [100 700 120 720] /Contents (Note) /Popup 5 0 R >>",
        "<< /Type /Annot /Subtype /Popup /Rect [130 600 300 720] /Parent 4 0 R >>",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    write_pdf(&noted, "1.7", &objects, "");
    assert_eq!(engine.annotations(&noted, 0, &[]).unwrap().len(), 2);
    assert!(engine
        .annotations(
            &noted,
            0,
            &[PdfEdit::DeleteAnnotation { page: 0, index: 0 }]
        )
        .unwrap()
        .is_empty());
    assert!(engine
        .annotations(
            &noted,
            0,
            &[PdfEdit::DeleteAnnotation { page: 0, index: 2 }]
        )
        .is_err());
}

/// Two pages of fields. Page 0 annotation order: Name, Email, Agree, radio S,
/// radio M, Color (combo), Fruits (multi-select list). Page 1: Notes.
fn form_pdf(path: &std::path::Path, xfa: bool) {
    let field = |rest: &str| {
        format!("<< /Type /Annot /Subtype /Widget /F 4 /DA (/Helv 12 Tf 0 g) {rest} >>")
    };
    let objects: Vec<String> = vec![
        format!(
            "<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R {} >>",
            if xfa { "/NeedsRendering true" } else { "" }
        ),
        "<< /Type /Pages /Count 2 /Kids [3 0 R 4 0 R] >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [6 0 R 7 0 R 8 0 R 10 0 R 11 0 R 12 0 R 13 0 R] /Resources << /Font << /Helv 14 0 R >> >> >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [15 0 R] /Resources << /Font << /Helv 14 0 R >> >> >>".into(),
        format!(
            "<< /Fields [6 0 R 7 0 R 8 0 R 9 0 R 12 0 R 13 0 R 15 0 R] /DA (/Helv 12 Tf 0 g) /DR << /Font << /Helv 14 0 R >> >> {} >>",
            if xfa { "/XFA (unused)" } else { "" }
        ),
        field("/FT /Tx /T (Name) /V () /Rect [60 700 300 724] /P 3 0 R"),
        field("/FT /Tx /T (Email) /V () /Rect [60 660 300 684] /P 3 0 R"),
        field("/FT /Btn /T (Agree) /V /Off /AS /Off /Rect [60 620 80 640] /P 3 0 R /AP << /N << /Yes 16 0 R /Off 17 0 R >> >>"),
        "<< /FT /Btn /Ff 49152 /T (Size) /V /Off /Kids [10 0 R 11 0 R] >>".into(),
        field("/Parent 9 0 R /AS /Off /Rect [60 580 80 600] /P 3 0 R /AP << /N << /S 16 0 R /Off 17 0 R >> >>"),
        field("/Parent 9 0 R /AS /Off /Rect [100 580 120 600] /P 3 0 R /AP << /N << /M 16 0 R /Off 17 0 R >> >>"),
        field("/FT /Ch /Ff 131072 /T (Color) /Opt [(Red) (Green) (Blue)] /V (Red) /Rect [60 540 200 560] /P 3 0 R"),
        field("/FT /Ch /Ff 2097152 /T (Fruits) /Opt [(Apple) (Banana) (Cherry)] /Rect [60 440 200 520] /P 3 0 R"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        field("/FT /Tx /T (Notes) /V () /Rect [60 700 300 724] /P 4 0 R"),
        stream("/Type /XObject /Subtype /Form /BBox [0 0 20 20]", "0 0 m 20 20 l 0 20 m 20 0 l 2 w 0 0 0 RG S"),
        stream("/Type /XObject /Subtype /Form /BBox [0 0 20 20]", ""),
    ];
    write_pdf(path, "1.7", &objects, "");
}

#[test]
fn inline_form_session_fills_every_field_kind_and_commits_recipes() {
    use model::FormInput::*;
    let dir = out_dir("forms");
    let path = dir.join("form.pdf");
    form_pdf(&path, false);
    let mut engine = PdfEngine::new().unwrap();
    let mut edits: Vec<PdfEdit> = Vec::new();
    let send =
        |engine: &mut PdfEngine, page: u32, input: model::FormInput, edits: &mut Vec<PdfEdit>| {
            let feedback = engine.form_event(&path, page, input, edits).unwrap();
            edits.extend(feedback.commits.iter().cloned());
            feedback
        };
    let click = |engine: &mut PdfEngine, edits: &mut Vec<PdfEdit>, x: f32, y: f32| {
        send(engine, 0, PointerDown { x, y }, edits);
        send(engine, 0, PointerUp { x, y }, edits)
    };
    let fill = |edits: &[PdfEdit]| -> Vec<(u32, u32, String)> {
        edits
            .iter()
            .map(|e| match e {
                PdfEdit::FillField {
                    page,
                    annotation_index,
                    value,
                } => (*page, *annotation_index, value.clone()),
                other => panic!("unexpected {other:?}"),
            })
            .collect()
    };
    let tab = Key {
        code: 9,
        shift: false,
        ctrl: false,
        alt: false,
    };
    let back_tab = Key {
        code: 9,
        shift: true,
        ctrl: false,
        alt: false,
    };
    // Click into Name and type, with a backspace.
    let focus = click(&mut engine, &mut edits, 180.0, 80.0);
    let (page, rect) = focus.focus.unwrap();
    assert_eq!(page, 0);
    assert!(
        (rect[0] - 60.0 / 612.0).abs() < 0.01 && (rect[1] - 68.0 / 792.0).abs() < 0.01,
        "{rect:?}"
    );
    for c in "Adx".chars() {
        assert!(send(&mut engine, 0, Char(c), &mut edits).redraw);
    }
    send(&mut engine, 0, Char('\u{8}'), &mut edits);
    send(&mut engine, 0, Char('a'), &mut edits);
    assert!(
        edits.is_empty(),
        "typing must not commit before focus leaves"
    );
    // Tab commits Name and moves to Email. A render with the new recipe
    // keeps the session, so typing continues in Email.
    let moved = send(&mut engine, 0, tab, &mut edits);
    assert_eq!(fill(&edits), [(0, 0, "Ada".to_string())]);
    assert!((moved.focus.unwrap().1[1] - 108.0 / 792.0).abs() < 0.01);
    engine
        .render_region(&path, 0, 1.0, [0, 0, 612, 792], &edits)
        .unwrap();
    for c in "e@x.io".chars() {
        send(&mut engine, 0, Char(c), &mut edits);
    }
    let back = send(&mut engine, 0, back_tab, &mut edits);
    assert_eq!(fill(&edits)[1], (0, 1, "e@x.io".to_string()));
    assert!((back.focus.unwrap().1[1] - 68.0 / 792.0).abs() < 0.01);
    // Checkbox and radio commit on click.
    click(&mut engine, &mut edits, 70.0, 162.0);
    assert_eq!(fill(&edits)[2], (0, 2, "true".to_string()));
    click(&mut engine, &mut edits, 110.0, 202.0);
    assert_eq!(fill(&edits)[3], (0, 4, "true".to_string()));
    // Combo box: focus it, choose the next option with the keyboard, leave.
    click(&mut engine, &mut edits, 130.0, 242.0);
    let down = Key {
        code: 0x28,
        shift: false,
        ctrl: false,
        alt: false,
    };
    send(&mut engine, 0, Blur, &mut edits);
    click(&mut engine, &mut edits, 130.0, 242.0);
    send(&mut engine, 0, down, &mut edits);
    send(&mut engine, 0, Blur, &mut edits);
    // List box: click the second item.
    click(&mut engine, &mut edits, 100.0, 792.0 - 520.0 + 20.0);
    send(&mut engine, 0, Blur, &mut edits);
    let filled = fill(&edits);
    println!("form session commits: {filled:?}");
    assert!(filled.contains(&(0, 5, "Green".to_string())), "{filled:?}");
    assert!(filled.contains(&(0, 6, "Banana".to_string())), "{filled:?}");
    // Tab past the last field on page 0 moves to page 1.
    click(&mut engine, &mut edits, 100.0, 792.0 - 520.0 + 20.0);
    let next = send(&mut engine, 0, tab, &mut edits);
    assert_eq!(next.focus.unwrap().0, 1);
    for c in "Page two".chars() {
        send(&mut engine, 1, Char(c), &mut edits);
    }
    let previous = send(&mut engine, 1, back_tab, &mut edits);
    assert_eq!(previous.focus.unwrap().0, 0);
    send(&mut engine, 0, Blur, &mut edits);
    assert!(fill(&edits).contains(&(1, 0, "Page two".to_string())));
    // The recipe replays on a fresh document: saved values match.
    let saved = dir.join("filled.pdf");
    engine.save_copy(&path, &saved, &edits).unwrap();
    let fields = engine.form_fields(&saved, 0, &[]).unwrap();
    let values: Vec<&str> = fields.iter().map(|f| f.value.as_str()).collect();
    println!("saved field values: {values:?}");
    assert_eq!(&values[..3], ["Ada", "e@x.io", "true"]);
    assert_eq!(values[4], "M");
    assert_eq!(values[5], "Green");
    assert_eq!(
        engine.form_fields(&saved, 1, &[]).unwrap()[0].value,
        "Page two"
    );
    // Replay checks the value it sets.
    let bad = [PdfEdit::FillField {
        page: 0,
        annotation_index: 5,
        value: "Purple".into(),
    }];
    assert!(engine.render_edited(&path, 0, 100, 100, &bad).is_err());
    // XFA forms and PDFs without forms are refused.
    let xfa = dir.join("xfa.pdf");
    form_pdf(&xfa, true);
    assert_eq!(
        engine.form_event(&xfa, 0, Blur, &[]).unwrap_err(),
        pdf::XFA_FORM
    );
    assert_eq!(
        engine.metadata(&xfa, &[]).unwrap().form,
        model::PdfFormType::XfaFull
    );
    let plain = dir.join("plain.pdf");
    text_pdf(&plain, &[text_at(72.0, 700.0, 12.0, "No fields")]);
    assert!(engine.form_event(&plain, 0, Blur, &[]).is_err());
}

struct Com;
impl Com {
    fn new() -> Self {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap() };
        Com
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        unsafe { windows::Win32::System::Com::CoUninitialize() }
    }
}

/// An opaque image: `paint(x, y)` gives [blue, green, red].
fn image(width: u32, height: u32, paint: impl Fn(u32, u32) -> [u8; 3]) -> model::Frame {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            pixels.extend(paint(x, y));
            pixels.push(255);
        }
    }
    model::Frame {
        width,
        height,
        pixels,
        page_count: 1,
        source_width: width,
        source_height: height,
    }
}

fn pixel(frame: &model::Frame, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * frame.width + x) * 4) as usize;
    [frame.pixels[i], frame.pixels[i + 1], frame.pixels[i + 2]]
}

fn is_red(p: [u8; 3]) -> bool {
    p[2] > 180 && p[1] < 90 && p[0] < 90
}

const RED: [u8; 3] = [0, 0, 255];
const BLUE: [u8; 3] = [255, 0, 0];
const GRAY: [u8; 3] = [200, 200, 200];

/// Inserts an EXIF APP1 segment with `orientation` after the JPEG SOI marker.
fn with_orientation(jpeg: &[u8], orientation: u16) -> Vec<u8> {
    let mut tiff = b"II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0".to_vec();
    tiff.extend(orientation.to_le_bytes());
    tiff.extend([0, 0, 0, 0, 0, 0]);
    let mut app1 = b"Exif\0\0".to_vec();
    app1.extend(tiff);
    let mut out = jpeg[..2].to_vec();
    out.extend([0xFF, 0xE1]);
    out.extend(((app1.len() + 2) as u16).to_be_bytes());
    out.extend(app1);
    out.extend(&jpeg[2..]);
    out
}

#[test]
fn image_pages_keep_jpeg_bytes_orientation_and_page_size_rules() {
    let _com = Com::new();
    let dir = out_dir("images");
    // Landscape PNG: red left half, blue right half.
    let png = dir.join("wide.png");
    imaging::export_frame(
        &image(400, 200, |x, _| if x < 200 { RED } else { BLUE }),
        &png,
    )
    .unwrap();
    // JPEG with the stored top-left quarter red and EXIF orientation 6
    // (shown turned clockwise), so the red quarter displays top right.
    let plain_jpeg = dir.join("plain.jpg");
    imaging::export_frame(
        &image(300, 200, |x, y| if x < 150 && y < 100 { RED } else { GRAY }),
        &plain_jpeg,
    )
    .unwrap();
    let jpeg = dir.join("turned.jpg");
    let jpeg_bytes = with_orientation(&std::fs::read(&plain_jpeg).unwrap(), 6);
    std::fs::write(&jpeg, &jpeg_bytes).unwrap();
    let decoded = imaging::decode(&jpeg, 1000, 1000).unwrap();
    assert_eq!(
        (decoded.width, decoded.height),
        (200, 300),
        "WIC applies EXIF"
    );
    assert!(is_red(pixel(&decoded, 150, 50)) && !is_red(pixel(&decoded, 50, 50)));

    let mut engine = PdfEngine::new().unwrap();
    let combined = dir.join("combined.pdf");
    engine
        .create_from_images(&[jpeg.clone(), png.clone()], &combined)
        .unwrap();
    // 96 DPI: 0.75 points per pixel, after the EXIF turn.
    assert_eq!(
        engine.page_sizes(&combined, &[]).unwrap(),
        vec![[150.0, 225.0], [300.0, 150.0]]
    );
    let page = engine.render(&combined, 0, 200, 300).unwrap();
    for (x, y) in [(150, 50), (50, 50), (50, 250), (150, 250)] {
        assert_eq!(
            is_red(pixel(&page, x, y)),
            is_red(pixel(&decoded, x, y)),
            "PDF and WIC disagree at ({x}, {y})"
        );
    }
    // The JPEG file is embedded byte for byte.
    let saved = std::fs::read(&combined).unwrap();
    let middle = &jpeg_bytes[jpeg_bytes.len() / 2..][..256];
    assert!(saved.windows(256).any(|w| w == middle));

    // Insert into a Letter PDF: the page takes the neighbor's size, turned
    // landscape for the wide image, which is centered at full width.
    let letter = dir.join("letter.pdf");
    text_pdf(
        &letter,
        &[
            text_at(72.0, 700.0, 12.0, "One"),
            text_at(72.0, 700.0, 12.0, "Two"),
        ],
    );
    let insert = [PdfEdit::InsertImage {
        at: 1,
        path: png.clone(),
    }];
    assert_eq!(
        engine.page_sizes(&letter, &insert).unwrap(),
        vec![[612.0, 792.0], [792.0, 612.0], [612.0, 792.0]]
    );
    let inserted = engine.render_edited(&letter, 1, 792, 612, &insert).unwrap();
    assert!(is_red(pixel(&inserted, 100, 306)));
    assert!(pixel(&inserted, 700, 306)[0] > 180 && pixel(&inserted, 700, 306)[2] < 90);
    assert_eq!(pixel(&inserted, 400, 40), [255, 255, 255]);
    assert!(engine
        .page_text_edited(&letter, 2, &insert)
        .unwrap()
        .contains("Two"));
    let out = dir.join("inserted.pdf");
    engine.save_copy(&letter, &out, &insert).unwrap();
    assert_eq!(engine.page_sizes(&out, &[]).unwrap().len(), 3);

    assert!(engine
        .create_from_images(&[], &dir.join("none.pdf"))
        .is_err());
    let broken = dir.join("broken.png");
    std::fs::write(&broken, b"not an image").unwrap();
    assert!(engine
        .create_from_images(&[broken], &dir.join("broken.pdf"))
        .is_err());
    assert!(!dir.join("broken.pdf").exists());
}

#[test]
fn merge_flattens_forms_only_when_asked() {
    let dir = out_dir("merge");
    let form = dir.join("form.pdf");
    acroform_pdf(&form);
    let other = dir.join("other.pdf");
    text_pdf(&other, &[text_at(72.0, 700.0, 12.0, "Second document")]);
    let mut engine = PdfEngine::new().unwrap();
    let refused = dir.join("refused.pdf");
    assert!(engine.merge(&form, &other, &refused, &[]).is_err());
    assert!(engine
        .merge_with(&form, &other, &refused, &[], false)
        .is_err());
    assert!(!refused.exists());
    let merged = dir.join("merged.pdf");
    engine
        .merge_with(&form, &other, &merged, &[], true)
        .unwrap();
    let info = engine.metadata(&merged, &[]).unwrap();
    assert_eq!((info.page_count, info.form), (2, model::PdfFormType::None));
    assert!(engine
        .page_text(&merged, 0)
        .unwrap()
        .contains("Original value"));
    assert!(engine
        .page_text(&merged, 1)
        .unwrap()
        .contains("Second document"));
    assert!(engine.annotations(&merged, 0, &[]).unwrap().is_empty());
    // A filled value is what gets flattened.
    let filled = dir.join("filled.pdf");
    let fill = [PdfEdit::FillField {
        page: 0,
        annotation_index: 0,
        value: "Filled in".into(),
    }];
    engine
        .merge_with(&form, &other, &filled, &fill, true)
        .unwrap();
    assert!(engine.page_text(&filled, 0).unwrap().contains("Filled in"));
    let xfa = dir.join("xfa.pdf");
    form_pdf(&xfa, true);
    assert_eq!(
        engine
            .merge_with(&xfa, &other, &dir.join("xfa-merged.pdf"), &[], true)
            .unwrap_err(),
        pdf::XFA_FORM
    );
}

/// A small PDF whose cross-reference is a stream (PDF 1.5), as most modern
/// writers produce.
fn xref_stream_pdf(path: &std::path::Path) {
    let content = text_at(72.0, 700.0, 12.0, "Cross-reference stream");
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Count 2 /Kids [3 0 R 6 0 R] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        stream("", &content),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> >>".to_string(),
    ];
    let mut data = b"%PDF-1.5\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in bodies.iter().enumerate() {
        offsets.push(data.len());
        data.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).bytes());
    }
    let xref = data.len();
    let mut entries = vec![0u8, 0, 0, 0, 0, 0xFF, 0xFF];
    for offset in offsets.iter().chain(std::iter::once(&xref)) {
        entries.push(1);
        entries.extend((*offset as u32).to_be_bytes());
        entries.extend([0, 0]);
    }
    data.extend(
        format!(
            "7 0 obj\n<< /Type /XRef /Size 8 /W [1 4 2] /Root 1 0 R /Length {} >>\nstream\n",
            entries.len()
        )
        .bytes(),
    );
    data.extend(entries);
    data.extend(format!("\nendstream\nendobj\nstartxref\n{xref}\n%%EOF\n").bytes());
    std::fs::write(path, data).unwrap();
}

#[test]
fn incremental_save_appends_only_the_update() {
    let _com = Com::new();
    let dir = out_dir("incremental");
    let source = fixture("500-pages-50mb.pdf");
    let original = std::fs::read(&source).unwrap();
    let highlight = PdfEdit::Annotate {
        page: 0,
        kind: AnnotationKind::Highlight,
        points: vec![[0.08, 0.1], [0.6, 0.14]],
        text: String::new(),
    };
    let png = dir.join("page.png");
    imaging::export_frame(&image(200, 100, |_, _| RED), &png).unwrap();
    let mut engine = PdfEngine::new().unwrap();
    // A recipe, then a check of the saved page order: (page, expected text).
    let cases: Vec<(&str, Vec<PdfEdit>, Vec<(u32, &str)>)> = vec![
        (
            "highlight",
            vec![highlight.clone()],
            vec![(0, "page 1 of"), (499, "page 500 of")],
        ),
        (
            "move",
            vec![PdfEdit::Move { from: 499, to: 0 }],
            vec![(0, "page 500 of"), (1, "page 1 of")],
        ),
        (
            "delete",
            vec![PdfEdit::Delete { page: 10 }],
            vec![(10, "page 12 of"), (498, "page 500 of")],
        ),
        (
            "blank",
            vec![PdfEdit::InsertBlank { at: 5 }],
            vec![(5, ""), (6, "page 6 of")],
        ),
        (
            "image",
            vec![PdfEdit::InsertImage {
                at: 2,
                path: png.clone(),
            }],
            vec![(2, ""), (3, "page 3 of")],
        ),
        (
            "mixed",
            vec![
                PdfEdit::Move { from: 3, to: 0 },
                highlight.clone(),
                PdfEdit::Delete { page: 1 },
                PdfEdit::RotateRight { page: 0 },
            ],
            vec![(0, "page 4 of"), (1, "page 2 of")],
        ),
    ];
    for (name, edits, checks) in &cases {
        let output = dir.join(format!("{name}.pdf"));
        let start = Instant::now();
        let mode = engine.save_incremental(&source, edits, &output).unwrap();
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        let saved = std::fs::read(&output).unwrap();
        assert_eq!(mode, model::SaveMode::Incremental, "{name}");
        assert!(
            saved.starts_with(&original),
            "{name}: original bytes changed"
        );
        let added = saved.len() - original.len();
        println!(
            "incremental save {name}: {added} bytes added to {} in {ms:.0} ms",
            original.len()
        );
        // Only the touched objects and the page tree: never page contents.
        assert!(added < 100_000, "{name}: {added} bytes added");
        let expected = engine.page_sizes(&source, edits).unwrap();
        assert_eq!(engine.page_sizes(&output, &[]).unwrap(), expected, "{name}");
        for (page, text) in checks {
            let actual = engine.page_text(&output, *page).unwrap();
            assert!(actual.contains(text), "{name}: page {page} has {actual:?}");
        }
        if let Some(ok) = qpdf_check(&output) {
            assert!(ok, "{name}: qpdf --check failed");
        } else {
            eprintln!("qpdf not found: skipped qpdf --check");
        }
    }
    let annotated = dir.join("highlight.pdf");
    assert_eq!(
        engine.annotations(&annotated, 0, &[]).unwrap()[0].kind,
        "Highlight"
    );
    // PDFium's update for a cross-reference-stream source is damaged, so
    // the engine writes a full copy instead.
    let modern = dir.join("modern.pdf");
    xref_stream_pdf(&modern);
    if let Some(ok) = qpdf_check(&modern) {
        assert!(ok, "the generated xref-stream source is invalid");
    }
    let modern_out = dir.join("modern-saved.pdf");
    let edits = [highlight, PdfEdit::Move { from: 1, to: 0 }];
    let mode = engine
        .save_incremental(&modern, &edits, &modern_out)
        .unwrap();
    assert_eq!(mode, model::SaveMode::Full);
    assert!(engine
        .page_text(&modern_out, 1)
        .unwrap()
        .contains("Cross-reference stream"));
    assert_eq!(
        engine.annotations(&modern_out, 1, &[]).unwrap()[0].kind,
        "Highlight"
    );
    if let Some(ok) = qpdf_check(&modern_out) {
        assert!(
            ok,
            "full save of the xref-stream source: qpdf --check failed"
        );
    }
    // Never overwrite.
    assert!(engine.save_incremental(&source, &[], &annotated).is_err());
}

#[test]
fn every_markup_kind_shows_in_pdfjs() {
    use AnnotationKind::*;
    let dir = out_dir("markup");
    let path = dir.join("plain.pdf");
    let mut content = String::new();
    for i in 0..9 {
        content += &text_at(
            60.0,
            740.0 - i as f32 * 72.0,
            14.0,
            "Text under the markup on this row",
        );
    }
    text_pdf(&path, &[content]);
    let kinds = [
        Ink, Highlight, Underline, Strikeout, Note, Rectangle, Ellipse, Arrow, Text,
    ];
    let edits: Vec<PdfEdit> = kinds
        .iter()
        .enumerate()
        .map(|(i, &kind)| {
            let top = 0.05 + i as f32 * 0.091;
            PdfEdit::Annotate {
                page: 0,
                kind,
                points: if kind == Ink {
                    vec![[0.1, top], [0.3, top + 0.04], [0.5, top], [0.7, top + 0.04]]
                } else {
                    vec![[0.1, top], [0.7, top + 0.04]]
                },
                text: format!("{kind:?} note"),
            }
        })
        .collect();
    let mut engine = PdfEngine::new().unwrap();
    let saved = dir.join("markup.pdf");
    engine.save_copy(&path, &saved, &edits).unwrap();
    let list = engine.annotations(&saved, 0, &[]).unwrap();
    let subtypes: Vec<&str> = list.iter().map(|a| a.kind.as_str()).collect();
    assert_eq!(
        subtypes,
        [
            "Ink",
            "Highlight",
            "Underline",
            "StrikeOut",
            "Text",
            "Square",
            "Circle",
            "Ink",
            "FreeText"
        ]
    );
    let bytes = std::fs::read(&saved).unwrap();
    let appearances = bytes.windows(4).filter(|w| w == b"/AP ").count()
        + bytes.windows(4).filter(|w| w == b"/AP<").count();
    assert!(appearances >= 9, "{appearances} appearance streams");
    // Independent renderer: pdf.js must draw each annotation where it is.
    let checker = root().join("tools/pdf-check");
    if !checker.join("node_modules/pdfjs-dist").is_dir() {
        eprintln!("pdf.js not installed: run npm install in tools/pdf-check");
        return;
    }
    let rects: Vec<String> = kinds
        .iter()
        .zip(&list)
        .map(|(kind, a)| {
            format!(
                "{{\"page\":0,\"name\":\"{kind:?}\",\"rect\":[{},{},{},{}]}}",
                a.rect[0], a.rect[1], a.rect[2], a.rect[3]
            )
        })
        .collect();
    let rects_path = dir.join("rects.json");
    std::fs::write(&rects_path, format!("[{}]", rects.join(","))).unwrap();
    let output = std::process::Command::new("node")
        .arg(checker.join("check.mjs"))
        .arg(&saved)
        .arg(&rects_path)
        .current_dir(&checker)
        .output();
    let Ok(output) = output else {
        eprintln!("Node.js not found: skipped the pdf.js check");
        return;
    };
    let report = String::from_utf8_lossy(&output.stdout);
    println!("pdf.js check: {report}");
    eprintln!("{}", String::from_utf8_lossy(&output.stderr));
    assert!(
        output.status.success(),
        "pdf.js did not draw every annotation"
    );
}
