// Incremental save, merge with optional form flattening, and images to PDF.
// APIs: fpdf_save.h (FPDF_SaveAsCopy with FPDF_INCREMENTAL), fpdf_ppo.h
// (FPDF_ImportPagesByIndex), fpdf_flatten.h (FPDFPage_Flatten).
use super::*;
use crate::model::SaveMode;

impl PdfEngine {
    /// Writes `output` as the original file's bytes, unchanged, plus an
    /// incremental update with `edits`. The edits replay on a fresh document,
    /// because PDFium appends every object a document has loaded, not only
    /// changed ones (docs/research/pdf-engine.md, section 2).
    ///
    /// The update must keep the original bytes, end in a classic xref table,
    /// and reopen with the same pages. Otherwise this writes a full copy and
    /// returns `SaveMode::Full`. PDFium's update for sources with a
    /// cross-reference stream fails qpdf --check (its XRef stream has no
    /// /Type and no endobj), so those sources always get a full copy.
    pub fn save_incremental(
        &mut self,
        path: &Path,
        edits: &[PdfEdit],
        output: &Path,
    ) -> Result<SaveMode, String> {
        let document = self.open(path, edits)?;
        self.reject_protected_export(document.native.handle)?;
        let output = std::path::absolute(output).map_err(|e| e.to_string())?;
        if output.try_exists().map_err(|e| e.to_string())? {
            return Err("Choose a new file name. Existing files are never overwritten.".into());
        }
        let parent = output.parent().ok_or("Choose a valid output folder.")?;
        let (temporary, file) = temporary_in(parent)?;
        self.write_file(document.native.handle, file, INCREMENTAL_SAVE)?;
        let checked = appended_update(&temporary.0, path)
            .map_err(|e| format!("Cannot check the saved PDF: {e}"))?;
        if checked && self.same_pages(document.native.handle, &temporary.0) {
            commit(&temporary, &output)?;
            return Ok(SaveMode::Incremental);
        }
        drop(temporary);
        self.write_new(document.native.handle, &output, FULL_SAVE)?;
        Ok(SaveMode::Full)
    }

    /// Like `merge`, but `flatten_forms` turns form fields into page content
    /// first, so their values stay visible after the merge. FPDFPage_Flatten
    /// also flattens other annotations on those pages. Without the option,
    /// PDFs with forms are refused, because a merge drops their fields.
    pub fn merge_with(
        &mut self,
        path: &Path,
        other: &Path,
        output: &Path,
        edits: &[PdfEdit],
        flatten_forms: bool,
    ) -> Result<(), String> {
        let mut first = self.open(path, edits)?;
        let mut second = self.open(other, &[])?;
        for source in [&mut first, &mut second] {
            self.reject_protected_export(source.native.handle)?;
            match unsafe { (self.api.form_type)(source.native.handle) } {
                0 => {}
                1 if flatten_forms => self.flatten(source)?,
                1 => self.reject_forms(source.native.handle)?,
                _ => return Err(XFA_FORM.into()),
            }
        }
        let destination = self.new_document()?;
        unsafe {
            for source in [first.native.handle, second.native.handle] {
                let count = (self.api.count)(source);
                if count <= 0 {
                    return Err("Cannot merge a PDF without pages.".into());
                }
                let index = (self.api.count)(destination.handle);
                if (self.api.import)(destination.handle, source, std::ptr::null(), 0, index) == 0 {
                    return Err("Could not merge these PDF pages.".into());
                }
            }
        }
        self.write_new(destination.handle, output, FULL_SAVE)
    }

    /// Makes a new PDF with one page per image. Each page is the image size
    /// at 96 DPI. JPEG files keep their original bytes. Initialize COM on
    /// this thread first.
    #[allow(dead_code)] // tests/pdf_engine.rs uses this.
    pub fn create_from_images(&mut self, images: &[PathBuf], output: &Path) -> Result<(), String> {
        if images.is_empty() {
            return Err("Choose at least one image.".into());
        }
        let document = self.new_document()?;
        for (index, image) in images.iter().enumerate() {
            std::fs::read(image)
                .map_err(|e| format!("Cannot read the image file: {e}"))
                .and_then(|bytes| {
                    let fit = pages::ImageFit::Natural;
                    self.insert_image(document.handle, index as u32, &bytes, fit)
                })
                .map_err(|e| format!("{e} ({})", image.display()))?;
        }
        self.write_new(document.handle, output, FULL_SAVE)
    }
}

impl PdfEngine {
    /// Imports every page of the PDF in `bytes` at `at`. A page import drops
    /// form fields, so PDFs with forms are refused, as in `merge`.
    pub(super) fn insert_pdf(&self, document: Handle, at: u32, bytes: &[u8]) -> Result<(), String> {
        let size = i32::try_from(bytes.len()).map_err(|_| "This PDF is too large to insert.")?;
        // FPDF_ImportPagesByIndex deep-copies the pages, so the source can
        // close (and its buffer go) once the import returns.
        let source = unsafe { (self.api.load_memory)(bytes.as_ptr(), size, std::ptr::null()) };
        if source.is_null() {
            return Err(match unsafe { (self.api.error)() } {
                4 => "This PDF is protected. Remove its password, then insert it.",
                _ => "This PDF is damaged or uses an unsupported format.",
            }
            .into());
        }
        let source = NativeHandle {
            handle: source,
            close: self.api.close_doc,
        };
        self.reject_protected_export(source.handle)?;
        self.reject_forms(source.handle)?;
        unsafe {
            let before = (self.api.count)(document);
            let added = (self.api.count)(source.handle);
            if added <= 0 {
                return Err("This PDF has no pages to insert.".into());
            }
            if (self.api.import)(document, source.handle, std::ptr::null(), 0, at as i32) == 0
                || (self.api.count)(document) != before + added
            {
                return Err("Cannot insert the PDF pages.".into());
            }
        }
        Ok(())
    }

    /// Flattens every page. Loading a page into the form environment first
    /// builds appearances for fields that lack them (NeedAppearances).
    fn flatten(&self, document: &mut Document) -> Result<(), String> {
        self.api.form(document)?;
        let count = unsafe { (self.api.count)(document.native.handle) }.max(0) as u32;
        for index in 0..count {
            let page = self.api.open_page(document, index)?;
            // FLAT_NORMALDISPLAY = 0; FLATTEN_FAIL = 0 (fpdf_flatten.h).
            if unsafe { (self.api.flatten)(page.handle, 0) } == 0 {
                return Err("Could not flatten the form fields. The original is unchanged.".into());
            }
        }
        Ok(())
    }

    /// True when `saved` reopens with the same page count and page sizes.
    fn same_pages(&self, document: Handle, saved: &Path) -> bool {
        let Ok(reopened) = self.open(saved, &[]) else {
            return false;
        };
        let sizes = |handle: Handle| -> Option<Vec<(f32, f32)>> {
            let count = unsafe { (self.api.count)(handle) };
            (0..count.max(0) as u32)
                .map(|i| self.api.page_size(handle, i).ok())
                .collect()
        };
        let expected = sizes(document);
        expected.is_some() && expected == sizes(reopened.native.handle)
    }
}

/// True when `saved` is `original` plus an update whose last `startxref`
/// points past the original bytes at a classic `xref` table.
fn appended_update(saved: &Path, original: &Path) -> std::io::Result<bool> {
    let length = std::fs::metadata(original)?.len();
    if !starts_with_file(saved, original)? {
        return Ok(false);
    }
    let mut file = File::open(saved)?;
    let size = file.metadata()?.len();
    let mut tail = vec![0; size.min(1024) as usize];
    file.seek(SeekFrom::Start(size - tail.len() as u64))?;
    file.read_exact(&mut tail)?;
    let Some(offset) = last_startxref(&tail) else {
        return Ok(false);
    };
    if offset < length || offset >= size {
        return Ok(false);
    }
    let mut keyword = [0; 4];
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(&mut keyword)?;
    Ok(&keyword == b"xref")
}

/// The number after the last `startxref` keyword.
fn last_startxref(tail: &[u8]) -> Option<u64> {
    let at = tail.windows(9).rposition(|w| w == b"startxref")?;
    let digits: String = tail[at + 9..]
        .iter()
        .skip_while(|b| b.is_ascii_whitespace())
        .take_while(|b| b.is_ascii_digit())
        .map(|&b| b as char)
        .collect();
    digits.parse().ok()
}

/// True when `file` begins with the bytes of `prefix`.
fn starts_with_file(file: &Path, prefix: &Path) -> std::io::Result<bool> {
    let (mut a, mut b) = (File::open(file)?, File::open(prefix)?);
    if a.metadata()?.len() < b.metadata()?.len() {
        return Ok(false);
    }
    let (mut x, mut y) = (vec![0; 1 << 20], vec![0; 1 << 20]);
    loop {
        let n = b.read(&mut y)?;
        if n == 0 {
            return Ok(true);
        }
        a.read_exact(&mut x[..n])?;
        if x[..n] != y[..n] {
            return Ok(false);
        }
    }
}
