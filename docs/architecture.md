# Architecture

Status: implemented development architecture, 2026-10-06. The owner delegated the language decision, removed the research-first sequence, and requested the usable app. The original PRD remains unchanged. There is no evidence that Rust beats a comparable NativeAOT app on the reference devices.

## Boundaries and first pixel

`shell.rs` owns the Win32 window, native buttons, menus, dialogs, tabs, input, and Direct2D presentation. A small window paints before file decoding. Jobs go to one COM-initialized worker. The worker opens a PDF with `pdf.rs` or decodes an image with `imaging.rs`. It returns owned premultiplied BGRA pixels. Native PDFium pointers and COM decoders never cross the channel. Generation IDs discard stale render/search/OCR results.

`pdf.rs` loads a pinned PDFium DLL using an absolute path. A process guard and a non-Send engine keep its native calls serialized. Documents, pages, bitmaps, form environments, and callbacks have explicit lifetimes. Document input uses a retained file callback rather than copying every PDF into memory. Source metadata checks reject stale edit recipes after external replacement.

`imaging.rs` builds a WIC pipeline from the original, EXIF orientation, ordered crops, resizes, transforms, and markup. Direct2D draws image annotations at source resolution. A separate display-size scale limits UI raster allocations. WebP has a permissive Rust encoder and a decoder fallback. HEIC uses the installed OS codec only.

`ocr.rs` invokes Windows OCR on the worker. `background.rs` initializes ONNX Runtime and its optional model on first request. CPU is the default because the actual DirectML probe failed on this host. `printing.rs` owns the print DC, page-range validation, rasterization, and cancellation. Its native dialog runs on the UI thread; rendering and spooling run on the worker.

## Save and undo

The UI keeps ordered edit recipes per source file. Undo removes the last edit. Revert clears the recipe. Exports reconstruct the source and apply the recipe, write a sibling temporary file, flush, validate, and publish a new filename without replacement. Existing files are not overwritten. PDF copies are full rewrites. Incremental in-place autosave and a crash-recovery journal are not implemented and are not implied by the Save Copy workflow.

Protected PDF outputs are refused until preservation of their encryption is proven. Copy and print permissions are checked. Forms cannot be silently lost by merge/extraction. Existing cryptographic signature validity is not promised for rewritten copies. Native ink signatures are visual annotations, not cryptographic signatures.

## Responsiveness and memory

UI rendering stays on the UI thread; file decode, PDF work, OCR, and segmentation do not. The worker currently serializes these operations, so a long AI operation delays later document work while the window remains interactive. A single display frame is capped at 64 MiB. Native decoder/model allocations are additional. This is not yet a bounded tile cache or neighbor-image prefetch design. Continuous multi-page scrolling, thumbnail virtualization, and cross-process parser isolation remain work.

## Errors and tests

No failed or blank decode emits a successful first-content marker. Invalid dimensions, corrupt data, missing codecs/models/languages, denied permissions, collisions, and native errors produce actionable errors. An independent agent reviews implementation. Tests reopen exported files and check page counts, text, form values, annotation pixels, alpha, dimensions, source preservation, and overwrite refusal. Desktop UI, physical printing, ARM64, and cross-reader tests are separate gates.

## Sources

- [Microsoft Rust Windows bindings](https://github.com/microsoft/windows-rs)
- [WIC overview](https://learn.microsoft.com/windows/win32/wic/-wic-about-windows-imaging-codec)
- [PDFium public API](https://pdfium.googlesource.com/pdfium/+/refs/heads/main/public/)
- [Windows OCR](https://learn.microsoft.com/uwp/api/windows.media.ocr.ocrengine)
- [DirectML execution provider constraints](https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html)
- [QPC measurement](https://learn.microsoft.com/windows/win32/sysinfo/acquiring-high-resolution-time-stamps)
