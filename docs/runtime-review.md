# Independent runtime review

Reviewer: test/UX agent. The reviewer did not implement the shell, PDF engine, WIC image pipeline, or background-removal pipeline. OCR and benchmark tools were written by this reviewer and were reviewed separately by the PDF agent.

## Checked behavior

Independent engine tests passed for WIC PNG alpha/pixel preservation, crop/rotate/resize, JPEG compositing onto white, source-resolution 24 MP PNG export, PDF edits and text operations, output collision refusal, AcroForm text fill/reopen, page merge/extract, standard annotation creation and saved appearance rendering. The original files remained byte-identical. See [test-plan.md](test-plan.md) and `tests/image_workflow.rs`, `tests/pdf_workflow.rs`.

The review checked PDF native ownership against the bundled pinned headers. Document callbacks retain stable boxed file/context pointers. Form sessions finish before page handles close. The current pinned `FPDFPage_InsertObject` returns `FPDF_BOOL` and takes ownership even on failure; the image import code follows that contract. This is source review and fixture evidence, not proof for every third-party PDF.

## Findings sent to implementers

| Finding | Latest verified state |
| --- | --- |
| PDF point dimensions were capped at one display pixel per point | Fixed with PDF-specific scaling; independent test checks rendering above 792 pixels |
| Startup used the 120 ms resize debounce | Source fix received; performance not remeasured after desktop interaction was stopped |
| Failed decode discarded prior frame | Shell source changed to retain it; final GUI behavior not verified |
| Search/copy ignored working PDF edits | Edited APIs integrated; engine edit-index tests passed |
| Keyboard shortcuts could miss focus on toolbar controls | Main message loop now handles shortcuts before child dispatch; final desktop verification remains pending |
| JPEG transparency could become black | Fixed with explicit white compositing; independent transparent-JPEG assertion passed |
| WIC output lacked durability/reopen checks | Added sync and reopened dimension validation; independent round trips passed |
| Background model lookup indexed an unchecked output name | Fixed with fallible `get` and an unsupported-pack error; source reviewed |
| JPEG scanline allocation can be large for extreme wide images | Fixed with `frame_bytes(width, 1)` before allocation; source reviewed |
| Single-point image ink saved no visible annotation | Guard confirmed by independent release test rerun |
| Image text annotation silently truncated UTF-16 | Source rejects text over its limit; independent release test passed |
| Image navigation replaces a tab containing unsaved edits | Source fix retains dirty tab and selects an existing destination; desktop behavior remains unverified |
| Delayed OCR/copy result has no document generation | Source fix checks generation and path before clipboard writes; desktop race test remains unverified |

## Unresolved acceptance limits

Existing cryptographic PDF signature validity is not proven after rewriting a copy. The original is preserved, but the UI must not imply that a copied/edited signature remains valid. Full AcroForm, XFA, Unicode annotation fonts, complex real PDFs, accessibility, and annotation compatibility with Edge/Acrobat remain separate acceptance work.

The 64 MiB frame contract permits at most 16,777,216 BGRA pixels. Full-resolution image-to-PDF and background removal must reject larger images explicitly or adopt another allocation contract; they must not silently export a display preview.

Background-removal CPU timing supplied by the coordinator exceeds the PRD's 3-second target. No quality parity or performance pass is claimed. Model mask shape and finite-value checks are present; they do not prove model accuracy.

The user stopped desktop interaction. No further GUI, viewer, or launch performance tests were performed after that instruction. Initial launch samples describe an earlier executable and must not be used as current-build performance.

Printing received a separate source review: printer-DC ownership, cleanup/abort, page-range validation, asymmetric DPI geometry and alpha compositing have no identified source blocker. Native print-dialog behavior, driver cancellation and physical output remain unverified.

Sources: [PDFium public headers](https://pdfium.googlesource.com/pdfium/+/refs/heads/main/public/), [WIC](https://learn.microsoft.com/windows/win32/wic/-wic-about-windows-imaging-codec), [ONNX Runtime](https://onnxruntime.ai/docs/), and the working-code/test evidence referenced above.


Independent final image rerun: release test image_workflow passed all 4 tests in 1.15 seconds. Added lossless WebP export and app decode preserved 2 by 2 premultiplied alpha pixels exactly and refused overwrite. The fallback decoder was source-reviewed; this does not prove a forced no-codec path.

Password/revision source review: successful password validation precedes cache replacement, protected export is refused, text/print permissions are enforced, and dirty recipes check source metadata before render/save. Length and creation/modification time are change indicators, not a content hash. Password cleanup wipes active stored bytes; it does not prove erasure of all UI or allocator copies. Password test evidence comes from the separate PDF implementer.

