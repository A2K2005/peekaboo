# Architecture

Status: implemented development architecture, 2026-10-06. The owner delegated the language decision, removed the research-first sequence, and requested the usable app. The original PRD remains unchanged. There is no evidence that Rust beats a comparable NativeAOT app on the reference devices.

## Boundaries and first pixel

`shell.rs` owns the Win32 window, native buttons, menus, dialogs, tabs, input, and Direct2D presentation. A small window paints before file decoding. Jobs go to one COM-initialized worker. The worker opens a PDF with `pdf.rs` or decodes an image with `imaging.rs`. It returns owned premultiplied BGRA pixels. Native PDFium pointers and COM decoders never cross the channel. Generation IDs discard stale render/search/OCR results.

`pdf.rs` loads a pinned PDFium DLL using an absolute path. A process guard and a non-Send engine keep its native calls serialized. Documents, pages, bitmaps, form environments, and callbacks have explicit lifetimes. Document input uses a retained file callback rather than copying every PDF into memory. Source metadata checks reject stale edit recipes after external replacement.

`imaging.rs` builds a WIC pipeline from the original, EXIF orientation, ordered crops, resizes, transforms, and markup. Direct2D draws image annotations at source resolution. A separate display-size scale limits UI raster allocations. WebP has a permissive Rust encoder and a decoder fallback. HEIC uses the installed OS codec only.

`ocr.rs` invokes Windows OCR on the worker. `background.rs` initializes ONNX Runtime and its optional model on first request. CPU is the default because the actual DirectML probe failed on this host. `printing.rs` owns the print DC, page-range validation, rasterization, and cancellation. Its native dialog runs on the UI thread; rendering and spooling run on the worker.

## Save and undo

The UI keeps ordered edit recipes and a revision-aware save session per logical source file. Before the first edit, the user chooses overwrite or Save a copy and can remember that policy. The worker creates one immutable snapshot of the exact opened bytes. Rendering, text, forms, printing, export, batch work, and autosave replay recipes from that snapshot so an overwrite cannot apply an edit twice. Undo removes the last edit. Revert clears the recipe and publishes the opened snapshot through the same safe-save path.

Autosave waits 500 ms after an edit. PDF saves use PDFium incremental output; image saves preserve identical source bytes when the format and recipe allow it, otherwise they export the edited frame. Every write is staged in the destination folder, flushed, rechecks the expected destination stamp, and then uses an atomic Windows replacement or a no-replace publish. Session and revision IDs reject stale completions. External changes pause autosave for an explicit overwrite, Save a copy, or keep-paused choice. Snapshot reader leases prevent tab, window, or Windows-session cleanup while work still reads a snapshot. Normal close removes snapshots; cleanup or recovery for snapshots left by a crash is not implemented.

Protected PDF outputs are refused until preservation of their encryption is proven. Copy and print permissions are checked. Forms cannot be silently lost by merge/extraction. Existing cryptographic signature validity is not promised for rewritten copies. Native ink signatures are visual annotations, not cryptographic signatures.

## Responsiveness and memory

UI rendering stays on the UI thread. One document worker serializes PDFium and snapshot-dependent WIC work; a task worker runs independent OCR, background removal, and batch jobs. A 48 MiB optional bitmap-cache budget bounds prefetched tiles while visible demand can temporarily exceed it. Visible PDF tiles and sidebar thumbnails take priority, old-scale fallback tiles do not pin the cache, and adjacent images predecode next before previous. Native decoder and model allocations remain additional. Process memory, 60 fps scrolling, and cold launch still need reference-device measurements. Cross-process parser isolation remains future hardening.

## Errors and tests

No failed or blank decode emits a successful first-content marker. Invalid dimensions, corrupt data, missing codecs/models/languages, denied permissions, collisions, and native errors produce actionable errors. An independent agent reviews implementation. Tests reopen exported files and check page counts, text, form values, annotation pixels, alpha, dimensions, source preservation, and overwrite refusal. Desktop UI, physical printing, ARM64, and cross-reader tests are separate gates.

## Sources

- [Microsoft Rust Windows bindings](https://github.com/microsoft/windows-rs)
- [WIC overview](https://learn.microsoft.com/windows/win32/wic/-wic-about-windows-imaging-codec)
- [PDFium public API](https://pdfium.googlesource.com/pdfium/+/refs/heads/main/public/)
- [Windows OCR](https://learn.microsoft.com/uwp/api/windows.media.ocr.ocrengine)
- [DirectML execution provider constraints](https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html)
- [QPC measurement](https://learn.microsoft.com/windows/win32/sysinfo/acquiring-high-resolution-time-stamps)
