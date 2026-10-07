#![allow(dead_code)]
#[path = "../src/imaging.rs"]
mod imaging;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/pdf.rs"]
mod pdf;

// Workflow check. Run serially because PDFium is process-global.
#[test]
fn pdf_edits_round_trip_without_changing_the_source() {
    use model::PdfEdit;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = root.join("fixtures/20-pages.pdf");
    assert!(source.is_file(), "Run tools/make-fixtures.ps1 first");
    let original = fs::read(&source).unwrap();
    let out = root.join("artifacts/pdf-workflow").join(format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&out).unwrap();
    let mut engine =
        pdf::PdfEngine::new().expect("Verified PDFium must be beside the profile executable");
    let first = engine.render(&source, 0, 1600, 1600).unwrap();
    assert_eq!(first.page_count, 20);
    assert!(
        first.height > 792,
        "PDF points must scale to the display resolution"
    );
    assert!(engine
        .page_text(&source, 0)
        .unwrap()
        .contains("Synthetic page 1 of 20"));
    let edits = [
        PdfEdit::Delete { page: 0 },
        PdfEdit::RotateRight { page: 0 },
    ];
    let modified = engine
        .render_edited(&source, 0, 1600, 1600, &edits)
        .unwrap();
    assert_eq!(modified.page_count, 19);
    assert!(modified.width > modified.height);
    assert!(engine
        .page_text_edited(&source, 0, &edits)
        .unwrap()
        .contains("Synthetic page 2 of 20"));
    assert_eq!(
        engine
            .find_edited(&source, "Synthetic page 3 of 20", 0, &edits)
            .unwrap(),
        Some(1)
    );
    let saved = out.join("edited.pdf");
    engine.save_copy(&source, &saved, &edits).unwrap();
    let reopened = engine.render(&saved, 0, 1600, 1600).unwrap();
    assert_eq!(reopened.page_count, 19);
    assert!(reopened.width > reopened.height);
    let saved_before = fs::read(&saved).unwrap();
    assert!(engine.save_copy(&source, &saved, &[]).is_err());
    assert_eq!(
        fs::read(&saved).unwrap(),
        saved_before,
        "Refused overwrite changed existing output"
    );
    let extracted = out.join("extracted.pdf");
    engine.extract_page(&source, &extracted, 0, &edits).unwrap();
    assert_eq!(
        engine.render(&extracted, 0, 1200, 1200).unwrap().page_count,
        1
    );
    assert!(engine
        .page_text(&extracted, 0)
        .unwrap()
        .contains("Synthetic page 2 of 20"));
    let merged = out.join("merged.pdf");
    engine.merge(&extracted, &source, &merged, &[]).unwrap();
    assert_eq!(
        engine.render(&merged, 0, 1200, 1200).unwrap().page_count,
        21
    );
    assert!(engine
        .page_text(&merged, 1)
        .unwrap()
        .contains("Synthetic page 1 of 20"));
    assert!(engine.render(&source, 20, 100, 100).is_err());
    assert!(engine
        .render(&root.join("fixtures/corrupt.pdf"), 0, 100, 100)
        .is_err());
    let deleted_all: Vec<_> = (0..20).map(|_| PdfEdit::Delete { page: 0 }).collect();
    assert!(engine
        .save_copy(&source, &out.join("empty.pdf"), &deleted_all)
        .is_err());
    assert!(!out.join("empty.pdf").exists());
    let form = root.join("fixtures/acroform-text.pdf");
    let fields = engine.form_fields(&form, 0, &[]).unwrap();
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].name, "Customer name");
    assert_eq!(fields[0].value, "Original value");
    let filled = out.join("filled.pdf");
    engine
        .save_copy(
            &form,
            &filled,
            &[PdfEdit::FillField {
                page: 0,
                annotation_index: fields[0].annotation_index,
                value: "Filled by test".into(),
            }],
        )
        .unwrap();
    assert_eq!(
        engine.form_fields(&filled, 0, &[]).unwrap()[0].value,
        "Filled by test"
    );
    assert_eq!(
        engine.form_fields(&form, 0, &[]).unwrap()[0].value,
        "Original value"
    );
    assert!(engine
        .merge(&form, &source, &out.join("unsafe-form-merge.pdf"), &[])
        .is_err());
    assert!(!out.join("unsafe-form-merge.pdf").exists());
    use model::AnnotationKind::*;
    let mut annotations = vec![];
    for (index, kind) in [
        Ink, Highlight, Underline, Strikeout, Note, Rectangle, Ellipse, Arrow, Text,
    ]
    .into_iter()
    .enumerate()
    {
        let top = 0.2 + index as f32 * 0.07;
        annotations.push(PdfEdit::Annotate {
            page: 0,
            kind,
            points: vec![[0.1, top], [0.6, top + 0.04]],
            text: "Review 123".into(),
        });
    }
    let annotated = out.join("annotations.pdf");
    engine.save_copy(&source, &annotated, &annotations).unwrap();
    let before = engine.render(&source, 0, 800, 800).unwrap();
    let after = engine.render(&annotated, 0, 800, 800).unwrap();
    assert_eq!(after.page_count, 20);
    assert!(
        before
            .pixels
            .iter()
            .zip(&after.pixels)
            .filter(|(a, b)| a != b)
            .count()
            > 100,
        "Saved annotation appearances did not render"
    );
    assert_eq!(
        fs::read(&source).unwrap(),
        original,
        "An operation changed the source"
    );
    assert!(
        !fs::read_dir(&out).unwrap().any(|e| e
            .unwrap()
            .path()
            .extension()
            .is_some_and(|x| x == "tmp")),
        "Temporary output leaked"
    );
}
