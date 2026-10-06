# Editing contracts

The owner removed the research-first sequence and Phase 3 stop on 2026-10-06. Build the usable application. Only concrete technical or licensing blockers justify deferral. HEIC patent exposure does not defer OCR or background removal generally.

## Images (coordinator implementation)

`model::ImageEdit` is `Clone`: `RotateRight`, `FlipHorizontal`, `Crop { left: f32, top: f32, right: f32, bottom: f32 }`, `Resize { width: u32, height: u32 }`.

Crop coordinates are normalized to the current oriented and edited image, each within 0..1. Edits run in order. `imaging::decode_edited(path, max_width, max_height, edits: &[ImageEdit]) -> Result<Frame,String>`. Existing `decode` delegates with an empty list.

`imaging::export(path: &Path, output: &Path, edits: &[ImageEdit]) -> Result<(),String>` decodes the original and applies edits at source resolution, never exports the display bitmap. PNG/JPEG/TIFF/BMP via WIC, selected from output extension. Output must not exist, including input path. The UI saves a copy to avoid unvalidated autosave/undo data loss. Default JPEG quality is 0.92. Export uses a sibling temporary file and commits only when fully encoded. `Resize` dimensions must be nonzero and at most 200 MP.

UI stores an edit Vec; undo pops, revert clears. Navigation with unsaved edits must ask save/discard/cancel or retain a per-file edit session. Crop is a drag gesture mapped from the displayed image rectangle into normalized coordinates. Resizing offers explicit width and height with an aspect-lock option.

## PDF (PDF agent implementation)

`PdfEngine::page_text(path, page_index) -> Result<String,String>` returns the engine's extracted text, not a guarantee of reading order.

`PdfEngine::find(path, query: &str, start_page: u32) -> Result<Option<u32>,String>` returns matching page, wraps once; no UI-thread work.

`model::PdfEdit`: `RotateRight { page: u32 }`, `Delete { page: u32 }`. Page indices are relative to the result of preceding edits.

`PdfEngine::render_edited(path, page, max_width, max_height, edits: &[PdfEdit]) -> Result<Frame,String>` applies edits on a retained private working document, or uses a fresh reload when recipe changes. `render` uses an empty recipe.

`PdfEngine::save_copy(path, output, edits: &[PdfEdit]) -> Result<(),String>` applies the recipe to a fresh document and writes a new PDF atomically. Never overwrite source. `extract_page(path, output, page, edits)` exports exactly the displayed page. `merge(path, other, output, edits)` imports pages into a new output; detect AcroForm documents and refuse merge if fields cannot be preserved. No flattening without explicit user choice.

File menu: save a copy, export current PDF page as image, extract page, merge PDF. Edit menu: rotate page, delete page with confirmation, undo, revert. Every queued mutation/export must be processed; only read-only render requests can be coalesced. Save completion is separate from render completion and cannot be discarded by generation filtering.

## Sources

- https://learn.microsoft.com/windows/win32/wic/-wic-creating-encoder
- https://pdfium.googlesource.com/pdfium/+/refs/heads/main/public/fpdf_edit.h
- https://pdfium.googlesource.com/pdfium/+/refs/heads/main/public/fpdf_save.h
- https://pdfium.googlesource.com/pdfium/+/refs/heads/main/public/fpdf_ppo.h
