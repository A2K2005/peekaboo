// Builds test PDFs. Tests generate what they need, so they do not depend on
// a shared fixtures/ folder. Output goes under artifacts/ (gitignored).
#![allow(dead_code)]
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A new, empty folder under artifacts/.
pub fn out_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = root()
        .join("artifacts/pdf-engine")
        .join(format!("{name}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes a PDF from objects numbered 1..=n. Object 1 is the catalog.
pub fn write_pdf(path: &Path, version: &str, objects: &[String], trailer: &str) {
    let mut data = format!("%PDF-{version}\n").into_bytes();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(data.len());
        data.extend(format!("{} 0 obj\n{object}\nendobj\n", index + 1).bytes());
    }
    let xref = data.len();
    data.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
    for offset in offsets {
        data.extend(format!("{offset:010} 00000 n \n").bytes());
    }
    data.extend(
        format!(
            "trailer\n<< /Root 1 0 R /Size {} {trailer} >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .bytes(),
    );
    fs::write(path, data).unwrap();
}

/// A stream object with the right /Length.
pub fn stream(dictionary: &str, content: &str) -> String {
    format!(
        "<< {dictionary} /Length {} >>\nstream\n{content}\nendstream",
        content.len()
    )
}

/// One text-showing operator: Helvetica (/F1) at `size` points, baseline at (x, y).
pub fn text_at(x: f32, y: f32, size: f32, text: &str) -> String {
    format!("BT /F1 {size} Tf {x} {y} Td ({text}) Tj ET\n")
}

/// A PDF with one page per `(media box, content stream)`.
pub fn pages_pdf(path: &Path, pages: &[(&str, String)]) {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        String::new(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut kids = String::new();
    for (media, content) in pages {
        let id = objects.len() + 1;
        kids.push_str(&format!("{id} 0 R "));
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox {media} /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
            id + 1
        ));
        objects.push(stream("", content));
    }
    objects[1] = format!("<< /Type /Pages /Count {} /Kids [{kids}] >>", pages.len());
    write_pdf(path, "1.7", &objects, "");
}

/// Letter pages with the given content streams.
pub fn text_pdf(path: &Path, pages: &[String]) {
    let pages: Vec<(&str, String)> = pages.iter().map(|c| ("[0 0 612 792]", c.clone())).collect();
    pages_pdf(path, &pages);
}

/// The same bytes tools/make-fixtures.ps1 writes for its synthetic PDFs.
pub fn synthetic_pdf(path: &Path, pages: usize, padding: usize) {
    let mut data = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![];
    let object = |data: &mut Vec<u8>, offsets: &mut Vec<usize>, id: usize, body: &str| {
        assert_eq!(id, offsets.len() + 1);
        offsets.push(data.len());
        data.extend(format!("{id} 0 obj\n{body}\nendobj\n").bytes());
    };
    object(
        &mut data,
        &mut offsets,
        1,
        "<< /Type /Catalog /Pages 2 0 R >>",
    );
    let kids: String = (0..pages).map(|p| format!("{} 0 R ", 4 + p * 2)).collect();
    object(
        &mut data,
        &mut offsets,
        2,
        &format!("<< /Type /Pages /Count {pages} /Kids [ {kids} ] >>"),
    );
    object(
        &mut data,
        &mut offsets,
        3,
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    );
    for p in 0..pages {
        let id = 4 + p * 2;
        object(&mut data, &mut offsets, id, &format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>", id + 1));
        let content = format!(
            "BT /F1 24 Tf 50 700 Td (Synthetic page {} of {pages}) Tj ET\n",
            p + 1
        );
        offsets.push(data.len());
        data.extend(
            format!(
                "{} 0 obj\n<< /Length {} >>\nstream\n{content}",
                id + 1,
                content.len() + padding
            )
            .bytes(),
        );
        data.resize(data.len() + padding, b' ');
        data.extend(b"\nendstream\nendobj\n");
    }
    let xref = data.len();
    data.extend(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).bytes());
    for offset in &offsets {
        data.extend(format!("{offset:010} 00000 n \n").bytes());
    }
    data.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len() + 1
        )
        .bytes(),
    );
    fs::write(path, data).unwrap();
}

/// A one-page form with a text field, like fixtures/acroform-text.pdf.
pub fn acroform_pdf(path: &Path) {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 6 0 R >>",
        "<< /Type /Pages /Count 1 /Kids [3 0 R] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [5 0 R] /Resources << /Font << /Helv 4 0 R >> >> >>",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (Customer name) /V (Original value) /Rect [50 650 350 700] /P 3 0 R /DA (/Helv 24 Tf 0 g) /F 4 >>",
        "<< /Fields [5 0 R] /DR << /Font << /Helv 4 0 R >> >> /DA (/Helv 24 Tf 0 g) /NeedAppearances true >>",
    ];
    let objects: Vec<String> = objects.iter().map(|s| s.to_string()).collect();
    write_pdf(path, "1.4", &objects, "");
}

/// fixtures/<name> when present; otherwise the same file generated under
/// artifacts/fixtures.
pub fn fixture(name: &str) -> PathBuf {
    let shared = root().join("fixtures").join(name);
    if shared.is_file() {
        return shared;
    }
    let local = root().join("artifacts/fixtures").join(name);
    if !local.is_file() {
        fs::create_dir_all(local.parent().unwrap()).unwrap();
        let partial = local.with_extension("partial");
        match name {
            "20-pages.pdf" => synthetic_pdf(&partial, 20, 0),
            "500-pages-50mb.pdf" => synthetic_pdf(&partial, 500, 100_000),
            "acroform-text.pdf" => acroform_pdf(&partial),
            "corrupt.pdf" => fs::write(&partial, "%PDF-1.7 broken xref").unwrap(),
            _ => panic!("No generator for fixture {name}"),
        }
        fs::rename(&partial, &local).unwrap();
    }
    local
}

/// Runs `qpdf --check` when qpdf is available (QPDF names the program,
/// else `qpdf` on PATH). Returns None when it is not installed.
pub fn qpdf_check(path: &Path) -> Option<bool> {
    let program = std::env::var("QPDF").unwrap_or_else(|_| "qpdf".into());
    let output = Command::new(program)
        .arg("--check")
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        eprintln!(
            "qpdf --check {}:\n{}{}",
            path.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Some(output.status.success())
}

/// Count of pixels that differ between two frames of the same size.
pub fn changed(a: &[u8], b: &[u8]) -> usize {
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(x, y)| {
            x.iter()
                .zip(y.iter())
                .map(|(p, q)| p.abs_diff(*q) as u32)
                .sum::<u32>()
                > 24
        })
        .count()
}

/// Dark pixels inside a normalized rectangle of a BGRA frame.
pub fn dark_in(pixels: &[u8], width: u32, height: u32, rect: [f32; 4]) -> usize {
    let (w, h) = (width as f32, height as f32);
    let mut count = 0;
    for y in (rect[1] * h) as u32..((rect[3] * h) as u32).min(height) {
        for x in (rect[0] * w) as u32..((rect[2] * w) as u32).min(width) {
            let p = &pixels[((y * width + x) * 4) as usize..][..3];
            if p.iter().map(|&v| v as u32).sum::<u32>() < 384 {
                count += 1;
            }
        }
    }
    count
}

/// The median of measured milliseconds.
pub fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}
