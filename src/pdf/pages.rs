// Page sizes, outline, metadata, annotation list and edits, and image pages.
// APIs: fpdfview.h (FPDF_GetPageSizeByIndexF, FPDF_GetFileVersion),
// fpdf_doc.h (FPDFBookmark_*, FPDFAction_*, FPDFDest_*, FPDF_GetMetaText),
// fpdf_annot.h (FPDFAnnot_*, FPDFPage_RemoveAnnot), and fpdf_edit.h
// (FPDFImageObj_LoadJpegFileInline, FPDFImageObj_SetBitmap).
use super::*;
use crate::model::{OutlineItem, PdfAnnotation, PdfFormType, PdfMetadata};
use std::collections::HashSet;

/// How an image page is sized.
pub(super) enum ImageFit {
    /// The size of the neighbor page, turned to the image orientation. The
    /// image is scaled to fit and centered. Used when inserting into a PDF.
    Neighbor,
    /// The image size at 96 DPI (1 px = 0.75 pt), the Windows default.
    /// Used when making a new PDF from images.
    Natural,
}

// PDF limits a page side to 14,400 units (ISO 32000-1, Annex C).
const MAX_PAGE_SIDE: f64 = 14_400.0;

const SUBTYPES: [&str; 29] = [
    "Unknown",
    "Text",
    "Link",
    "FreeText",
    "Line",
    "Square",
    "Circle",
    "Polygon",
    "PolyLine",
    "Highlight",
    "Underline",
    "Squiggly",
    "StrikeOut",
    "Stamp",
    "Caret",
    "Ink",
    "Popup",
    "FileAttachment",
    "Sound",
    "Movie",
    "Widget",
    "Screen",
    "PrinterMark",
    "TrapNet",
    "Watermark",
    "3D",
    "RichMedia",
    "XFAWidget",
    "Redact",
];

#[allow(dead_code)]
impl PdfEngine {
    /// The size in points of every page, after edits and rotation.
    pub fn page_sizes(&mut self, path: &Path, edits: &[PdfEdit]) -> Result<Vec<[f32; 2]>, String> {
        self.ensure(path, edits)?;
        let handle = self
            .document
            .as_ref()
            .ok_or("No PDF is open.")?
            .native
            .handle;
        let count = unsafe { (self.api.count)(handle) };
        if count < 0 {
            return Err("This PDF is damaged or uses an unsupported format.".into());
        }
        (0..count as u32)
            .map(|index| self.api.page_size(handle, index).map(|(w, h)| [w, h]))
            .collect()
    }

    /// The table of contents in display order. Entries whose page was
    /// deleted by an edit have no page.
    pub fn outline(&mut self, path: &Path, edits: &[PdfEdit]) -> Result<Vec<OutlineItem>, String> {
        self.ensure(path, edits)?;
        let handle = self
            .document
            .as_ref()
            .ok_or("No PDF is open.")?
            .native
            .handle;
        let mut items = Vec::new();
        let first = unsafe { (self.api.bookmark_child)(handle, std::ptr::null_mut()) };
        self.api
            .outline(handle, first, 0, &mut items, &mut HashSet::new());
        Ok(items)
    }

    pub fn metadata(&mut self, path: &Path, edits: &[PdfEdit]) -> Result<PdfMetadata, String> {
        self.ensure(path, edits)?;
        let handle = self
            .document
            .as_ref()
            .ok_or("No PDF is open.")?
            .native
            .handle;
        let api = &self.api;
        let text = |tag: &[u8]| {
            read_utf16(|buffer, length| unsafe {
                (api.meta_text)(handle, tag.as_ptr(), buffer.cast(), length)
            })
        };
        unsafe {
            let mut version = 0;
            let version = if (api.file_version)(handle, &mut version) != 0 && version > 0 {
                format!("{}.{}", version / 10, version % 10)
            } else {
                String::new()
            };
            Ok(PdfMetadata {
                title: text(b"Title\0"),
                author: text(b"Author\0"),
                subject: text(b"Subject\0"),
                keywords: text(b"Keywords\0"),
                creator: text(b"Creator\0"),
                producer: text(b"Producer\0"),
                created: text(b"CreationDate\0"),
                modified: text(b"ModDate\0"),
                version,
                page_count: (api.count)(handle).max(0) as u32,
                encrypted: (api.security_revision)(handle) >= 0,
                form: match (api.form_type)(handle) {
                    1 => PdfFormType::AcroForm,
                    2 => PdfFormType::XfaFull,
                    3 => PdfFormType::XfaForeground,
                    _ => PdfFormType::None,
                },
            })
        }
    }

    /// Every annotation on a page, including links and form widgets.
    pub fn annotations(
        &mut self,
        path: &Path,
        page: u32,
        edits: &[PdfEdit],
    ) -> Result<Vec<PdfAnnotation>, String> {
        self.ensure(path, edits)?;
        let handle = self
            .document
            .as_ref()
            .ok_or("No PDF is open.")?
            .native
            .handle;
        let page = self.api.page(handle, page)?;
        let display = self.api.display(page.handle)?;
        let count = unsafe { (self.api.annot_count)(page.handle) }.max(0) as u32;
        let mut list = Vec::with_capacity(count as usize);
        for index in 0..count {
            let annotation = self.api.annotation(page.handle, index)?;
            let mut rect = Rect::default();
            unsafe {
                let subtype = (self.api.annot_subtype)(annotation.handle);
                (self.api.annot_get_rect)(annotation.handle, &mut rect);
                list.push(PdfAnnotation {
                    index,
                    kind: SUBTYPES
                        .get(subtype.max(0) as usize)
                        .unwrap_or(&"Unknown")
                        .to_string(),
                    rect: display.norm(display.rect(
                        rect.left as f64,
                        rect.bottom as f64,
                        rect.right as f64,
                        rect.top as f64,
                    )),
                    contents: read_utf16(|buffer, length| {
                        (self.api.annot_get_string)(
                            annotation.handle,
                            b"Contents\0".as_ptr(),
                            buffer,
                            length,
                        )
                    }),
                });
            }
        }
        Ok(list)
    }
}

impl PdfEngine {
    pub(super) fn delete_annotation(&self, page: Handle, index: u32) -> Result<(), String> {
        let annotation = self.api.annotation(page, index)?;
        unsafe {
            if (self.api.annot_subtype)(annotation.handle) == 20 {
                return Err("Form fields cannot be deleted.".into());
            }
            // A note's popup belongs to it, so it goes too.
            let popup = (self.api.annot_linked)(annotation.handle, b"Popup\0".as_ptr());
            let popup = (!popup.is_null()).then(|| NativeHandle {
                handle: popup,
                close: self.api.annot_close,
            });
            let popup_index = popup
                .as_ref()
                .map(|p| (self.api.annot_index)(page, p.handle))
                .filter(|&i| i >= 0 && i as u32 != index);
            drop((popup, annotation));
            // Remove the higher index first so the lower one stays valid.
            let mut indices = vec![index as i32];
            indices.extend(popup_index);
            indices.sort_unstable_by(|a, b| b.cmp(a));
            for i in indices {
                if (self.api.annot_remove)(page, i) == 0 {
                    return Err("Cannot delete this annotation.".into());
                }
            }
        }
        Ok(())
    }

    pub(super) fn set_annotation_text(
        &self,
        page: Handle,
        index: u32,
        text: &str,
    ) -> Result<(), String> {
        if text.len() > 32_000 || text.contains('\0') {
            return Err("Annotation text is too long or contains invalid characters.".into());
        }
        let annotation = self.api.annotation(page, index)?;
        unsafe {
            let content = wide(text);
            if (self.api.annot_string)(annotation.handle, b"Contents\0".as_ptr(), content.as_ptr())
                == 0
            {
                return Err("Cannot change this annotation's text.".into());
            }
            // Free text shows its contents through /AP, so draw a new one.
            if (self.api.annot_subtype)(annotation.handle) == 3 {
                (self.api.annot_set_ap)(annotation.handle, 0, std::ptr::null());
                self.generate_appearance(page, annotation.handle)?;
            }
        }
        Ok(())
    }

    /// Adds an image file as a page at `at`. JPEG bytes are embedded as they
    /// are (DCTDecode), with the EXIF orientation applied by the image
    /// matrix. Other formats decode through WIC, so COM must be initialized
    /// on this thread.
    ///
    /// The page is built in a scratch document and imported. Generating page
    /// content inside `document` makes PDFium append every page's content
    /// stream to an incremental save (measured: 213 KB against 6 KB on the
    /// 500-page fixture).
    pub(super) fn insert_image(
        &self,
        document: Handle,
        at: u32,
        path: &Path,
        fit: ImageFit,
    ) -> Result<(), String> {
        let count = unsafe { (self.api.count)(document) }.max(0) as u32;
        let neighbor = match fit {
            ImageFit::Neighbor if count > 0 => Some(
                self.api
                    .page_size(document, at.saturating_sub(1).min(count - 1))?,
            ),
            _ => None,
        };
        let scratch = self.new_document()?;
        self.image_page(scratch.handle, path, neighbor)?;
        let first = 0;
        unsafe {
            if (self.api.import)(document, scratch.handle, &first, 1, at as i32) == 0
                || (self.api.count)(document) != count as i32 + 1
            {
                return Err("Cannot insert the image page.".into());
            }
        }
        Ok(())
    }

    /// Makes page 0 of an empty `document` from an image file. `neighbor`
    /// is the page size to fit into; `None` sizes the page to the image.
    fn image_page(
        &self,
        document: Handle,
        path: &Path,
        neighbor: Option<(f32, f32)>,
    ) -> Result<(), String> {
        let unreadable = |e: std::io::Error| format!("Cannot read the image file: {e}");
        let mut file = File::open(path).map_err(unreadable)?;
        let mut head = vec![0; 128 * 1024];
        let read = read_up_to(&mut file, &mut head).map_err(unreadable)?;
        head.truncate(read);
        let (image, width, height, orientation) = if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
            let (image, width, height) = self.jpeg_image(document, file)?;
            (image, width, height, exif_orientation(&head))
        } else {
            drop(file);
            let (frame, orientation) = decode_image(&std::fs::read(path).map_err(unreadable)?)?;
            let image = self.image_from_frame(document, &frame)?;
            (image, frame.width, frame.height, orientation)
        };
        // Width and height as displayed, after the EXIF rotation.
        let (width, height) = if orientation >= 5 {
            (height as f64, width as f64)
        } else {
            (width as f64, height as f64)
        };
        let (page_width, page_height, box_) = match neighbor {
            Some((w, h)) => {
                let (w, h) = (w as f64, h as f64);
                let (w, h) = if (width > height) != (w > h) {
                    (h, w)
                } else {
                    (w, h)
                };
                let scale = (w / width).min(h / height);
                let (iw, ih) = (width * scale, height * scale);
                (w, h, [(w - iw) / 2.0, (h - ih) / 2.0, iw, ih])
            }
            None => {
                let scale = 0.75f64.min(MAX_PAGE_SIDE / width.max(height));
                let (w, h) = (width * scale, height * scale);
                (w, h, [0.0, 0.0, w, h])
            }
        };
        let m = image_matrix(orientation, box_);
        unsafe {
            if (self.api.image_matrix)(image.handle, m[0], m[1], m[2], m[3], m[4], m[5]) == 0 {
                return Err("Cannot place the image on the page.".into());
            }
            let page = (self.api.page_new)(document, 0, page_width, page_height);
            if page.is_null() {
                return Err("Cannot insert the image page.".into());
            }
            let page = NativeHandle {
                handle: page,
                close: self.api.close_page,
            };
            let object = image.handle;
            std::mem::forget(image); // The page owns the object after insertion.
            if (self.api.page_insert)(page.handle, object) == 0
                || (self.api.page_generate)(page.handle) == 0
            {
                return Err("Cannot generate the image page.".into());
            }
        }
        Ok(())
    }

    /// An image object holding the JPEG file's bytes unchanged.
    fn jpeg_image(&self, document: Handle, file: File) -> Result<(NativeHandle, u32, u32), String> {
        let length = file.metadata().map_err(|e| e.to_string())?.len();
        if length == 0 || length > u32::MAX as u64 {
            return Err("The image file is empty or too large.".into());
        }
        let mut file = Box::new(file);
        let mut access = FileAccess {
            length: length as u32,
            read: read_block,
            context: (&mut *file as *mut File).cast(),
        };
        let image = self.new_image(document)?;
        let (mut width, mut height) = (0, 0);
        unsafe {
            if (self.api.jpeg_inline)(std::ptr::null_mut(), 0, image.handle, &mut access) == 0
                || (self.api.image_size)(image.handle, &mut width, &mut height) == 0
                || width == 0
                || height == 0
            {
                return Err("This JPEG image is damaged or unsupported.".into());
            }
        }
        Ok((image, width, height))
    }

    fn new_image(&self, document: Handle) -> Result<NativeHandle, String> {
        let handle = unsafe { (self.api.image_new)(document) };
        if handle.is_null() {
            return Err("Cannot create a PDF image object.".into());
        }
        Ok(NativeHandle {
            handle,
            close: self.api.object_destroy,
        })
    }

    /// An image object from premultiplied BGRA pixels. Opaque images carry
    /// no alpha mask.
    pub(super) fn image_from_frame(
        &self,
        document: Handle,
        frame: &Frame,
    ) -> Result<NativeHandle, String> {
        if frame_bytes(frame.width, frame.height)? != frame.pixels.len() {
            return Err("The image pixel buffer is invalid.".into());
        }
        let mut pixels = frame.pixels.clone();
        let opaque = pixels.chunks_exact(4).all(|p| p[3] == 255);
        if !opaque {
            for p in pixels.chunks_exact_mut(4) {
                let alpha = p[3] as u32;
                if alpha > 0 {
                    for channel in &mut p[..3] {
                        *channel = ((*channel as u32 * 255 + alpha / 2) / alpha).min(255) as u8;
                    }
                }
            }
        }
        unsafe {
            // FPDFBitmap_BGRx = 3, FPDFBitmap_BGRA = 4 (fpdfview.h).
            let bitmap = (self.api.bitmap)(
                frame.width as i32,
                frame.height as i32,
                if opaque { 3 } else { 4 },
                pixels.as_mut_ptr().cast(),
                (frame.width * 4) as i32,
            );
            if bitmap.is_null() {
                return Err("Cannot prepare the image for PDF export.".into());
            }
            let bitmap = NativeHandle {
                handle: bitmap,
                close: self.api.close_bitmap,
            };
            let image = self.new_image(document)?;
            if (self.api.image_bitmap)(std::ptr::null_mut(), 0, image.handle, bitmap.handle) == 0 {
                return Err("Cannot encode the image in PDF.".into());
            }
            Ok(image)
        }
    }
}

impl Api {
    fn outline(
        &self,
        document: Handle,
        mut node: Handle,
        level: u32,
        items: &mut Vec<OutlineItem>,
        seen: &mut HashSet<usize>,
    ) {
        // Damaged outlines can loop or nest without end.
        while !node.is_null() && level < 64 && items.len() < 100_000 && seen.insert(node as usize) {
            unsafe {
                let title =
                    read_utf16(|buffer, length| (self.bookmark_title)(node, buffer.cast(), length));
                let mut destination = (self.bookmark_dest)(document, node);
                if destination.is_null() {
                    let action = (self.bookmark_action)(node);
                    // PDFACTION_GOTO = 1 (fpdf_doc.h).
                    if !action.is_null() && (self.action_type)(action) == 1 {
                        destination = (self.action_dest)(document, action);
                    }
                }
                let page = if destination.is_null() {
                    None
                } else {
                    u32::try_from((self.dest_page)(document, destination)).ok()
                };
                items.push(OutlineItem { title, page, level });
                let child = (self.bookmark_child)(document, node);
                self.outline(document, child, level + 1, items, seen);
                node = (self.bookmark_sibling)(document, node);
            }
        }
    }
}

fn read_up_to(file: &mut File, buffer: &mut [u8]) -> std::io::Result<usize> {
    let mut total = 0;
    while total < buffer.len() {
        match file.read(&mut buffer[total..])? {
            0 => break,
            n => total += n,
        }
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(total)
}

const TOO_LARGE: &str = "Resize images above 16 megapixels before adding them to a PDF.";

/// Decodes a non-JPEG image at full size with WIC, and reads its EXIF
/// orientation. Images over the frame limit (64 MiB of pixels) are refused,
/// never scaled down. COM must be initialized on this thread.
fn decode_image(bytes: &[u8]) -> Result<(Frame, u16), String> {
    use windows::core::w;
    use windows::Win32::Graphics::Imaging::*;
    use windows::Win32::System::Com::{
        CoCreateInstance,
        StructuredStorage::{PropVariantClear, PropVariantToUInt16, PROPVARIANT},
        CLSCTX_INPROC_SERVER,
    };
    let damaged = |e: windows::core::Error| {
        format!(
            "This image is damaged or needs a Windows codec. {}",
            e.message()
        )
    };
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
                .map_err(damaged)?;
        let stream = factory.CreateStream().map_err(damaged)?;
        stream.InitializeFromMemory(bytes).map_err(damaged)?;
        let decoded = factory
            .CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)
            .and_then(|decoder| decoder.GetFrame(0));
        let frame = match decoded {
            Ok(frame) => frame,
            // Without the Windows WebP codec, imaging decodes WebP files itself.
            Err(_) if bytes.get(8..12) == Some(&b"WEBP"[..]) => {
                let file = Temporary(
                    std::env::temp_dir().join(format!(".preview-{}.webp", std::process::id())),
                );
                std::fs::write(&file.0, bytes)
                    .map_err(|e| format!("Cannot read the image file: {e}"))?;
                return Ok((crate::imaging::decode(&file.0, u32::MAX, u32::MAX)?, 1));
            }
            Err(e) => return Err(damaged(e)),
        };
        // TIFF orientation; JPEG files never reach this decoder.
        let mut orientation = 1;
        if let Ok(reader) = frame.GetMetadataQueryReader() {
            let mut value = PROPVARIANT::default();
            if reader
                .GetMetadataByName(w!("/ifd/{ushort=274}"), &mut value)
                .is_ok()
            {
                if let Ok(n @ 1..=8) = PropVariantToUInt16(&value) {
                    orientation = n;
                }
            }
            let _ = PropVariantClear(&mut value);
        }
        let (mut width, mut height) = (0, 0);
        frame.GetSize(&mut width, &mut height).map_err(damaged)?;
        let length = frame_bytes(width, height).map_err(|_| TOO_LARGE)?;
        let converter = factory.CreateFormatConverter().map_err(damaged)?;
        converter
            .Initialize(
                &frame,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
            .map_err(damaged)?;
        let mut pixels = vec![0; length];
        converter
            .CopyPixels(std::ptr::null(), width * 4, &mut pixels)
            .map_err(damaged)?;
        let image = Frame {
            width,
            height,
            pixels,
            page_count: 1,
            source_width: width,
            source_height: height,
        };
        Ok((image, orientation))
    }
}

/// The EXIF orientation (1 to 8) of a JPEG, or 1 when it has none.
pub(super) fn exif_orientation(jpeg: &[u8]) -> u16 {
    let mut at = 2;
    while at + 4 <= jpeg.len() && jpeg[at] == 0xFF {
        let marker = jpeg[at + 1];
        let length = u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]) as usize;
        if marker == 0xDA || length < 2 {
            break;
        }
        let body = jpeg.get(at + 4..at + 2 + length).unwrap_or(&[]);
        if marker == 0xE1 && body.starts_with(b"Exif\0\0") {
            return tiff_orientation(&body[6..]).unwrap_or(1);
        }
        at += 2 + length;
    }
    1
}

fn tiff_orientation(tiff: &[u8]) -> Option<u16> {
    let little = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |i: usize| -> Option<u16> {
        let b = [*tiff.get(i)?, *tiff.get(i + 1)?];
        Some(if little {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        })
    };
    let u32_at = |i: usize| -> Option<u32> {
        let b = [
            *tiff.get(i)?,
            *tiff.get(i + 1)?,
            *tiff.get(i + 2)?,
            *tiff.get(i + 3)?,
        ];
        Some(if little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    };
    let ifd = u32_at(4)? as usize;
    for entry in 0..u16_at(ifd)? as usize {
        let at = ifd + 2 + entry * 12;
        if u16_at(at)? == 0x0112 {
            return u16_at(at + 8).filter(|o| (1..=8).contains(o));
        }
    }
    None
}

/// The image matrix that draws an image with EXIF `orientation` upright in
/// `target` = `[x, y, width, height]` (page points, y up).
pub(super) fn image_matrix(orientation: u16, target: [f64; 4]) -> [f64; 6] {
    // Displayed position (s across, r down, both 0..1) as linear forms
    // `[constant, per a, per b]` of the image's unit square (a right, b up).
    let (s, r): ([f64; 3], [f64; 3]) = match orientation {
        2 => ([1.0, -1.0, 0.0], [1.0, 0.0, -1.0]),
        3 => ([1.0, -1.0, 0.0], [0.0, 0.0, 1.0]),
        4 => ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        5 => ([1.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
        6 => ([0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        7 => ([0.0, 0.0, 1.0], [1.0, -1.0, 0.0]),
        8 => ([1.0, 0.0, -1.0], [1.0, -1.0, 0.0]),
        _ => ([0.0, 1.0, 0.0], [1.0, 0.0, -1.0]),
    };
    let [x, y, w, h] = target;
    [
        w * s[1],
        -h * r[1],
        w * s[2],
        -h * r[2],
        x + w * s[0],
        y + h * (1.0 - r[0]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exif_jpeg(orientation: u16, little: bool) -> Vec<u8> {
        let mut tiff = Vec::new();
        let (w16, w32): (fn(u16) -> [u8; 2], fn(u32) -> [u8; 4]) = if little {
            (u16::to_le_bytes, u32::to_le_bytes)
        } else {
            (u16::to_be_bytes, u32::to_be_bytes)
        };
        tiff.extend(if little { b"II" } else { b"MM" });
        tiff.extend(w16(42));
        tiff.extend(w32(8));
        tiff.extend(w16(2));
        // An unrelated tag first, then Orientation (SHORT, count 1).
        tiff.extend(w16(0x010F));
        tiff.extend(w16(2));
        tiff.extend(w32(4));
        tiff.extend(w32(0));
        tiff.extend(w16(0x0112));
        tiff.extend(w16(3));
        tiff.extend(w32(1));
        tiff.extend(w16(orientation));
        tiff.extend([0, 0]);
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(tiff);
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xE1];
        jpeg.extend(((app1.len() + 2) as u16).to_be_bytes());
        jpeg.extend(app1);
        jpeg.extend([0xFF, 0xDA, 0, 2]);
        jpeg
    }

    #[test]
    fn reads_exif_orientation_in_both_byte_orders() {
        assert_eq!(exif_orientation(&exif_jpeg(6, true)), 6);
        assert_eq!(exif_orientation(&exif_jpeg(8, false)), 8);
        assert_eq!(exif_orientation(&exif_jpeg(9, true)), 1);
        assert_eq!(exif_orientation(&[0xFF, 0xD8, 0xFF, 0xDA, 0, 2]), 1);
        let mut cut = exif_jpeg(6, true);
        cut.truncate(30);
        assert_eq!(exif_orientation(&cut), 1);
    }

    #[test]
    fn image_matrix_puts_the_stored_top_left_where_exif_says() {
        // Map the stored image's top-left corner (a = 0, b = 1) to the page.
        let corner = |o| {
            let m = image_matrix(o, [10.0, 20.0, 100.0, 50.0]);
            (m[2] + m[4], m[3] + m[5])
        };
        assert_eq!(corner(1), (10.0, 70.0)); // top-left stays top-left
        assert_eq!(corner(3), (110.0, 20.0)); // rotated 180: bottom-right
        assert_eq!(corner(6), (110.0, 70.0)); // rotated 90 clockwise: top-right
        assert_eq!(corner(8), (10.0, 20.0)); // rotated 90 counterclockwise: bottom-left
        assert_eq!(corner(2), (110.0, 70.0)); // mirrored: top-right
    }
}
