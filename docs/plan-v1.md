# v1 build plan

Status: active, 2026-10-06. Owner: orchestrator. The owner asked to build every missing v1 feature end to end on the Rust build (D9). The PRD acceptance table is the definition of done. `docs/acceptance.md` tracks status per task.

## Process

1. Each slice has one implementer agent and one separate verifier agent. The verifier writes edge-case tests (large, corrupt, and password files; XFA; huge images; missing codecs) and reviews correctness, PRD criteria, accessibility, and code quality. It does not review its own code.
2. The orchestrator runs the benchmark harness after each slice. A regression over 10% against `docs/benchmarks.md` blocks the slice.
3. Wave 1 runs in parallel git worktrees with disjoint file ownership. The orchestrator merges. Waves 2 and 3 run on the merged tree.
4. A slice is done when its acceptance criteria pass, tests pass, and the verifier has no open blocking finding.
5. Small, focused commits. Commit messages follow the prose rules in `CLAUDE.md`.

## Shared contracts

These rules bind every wave-1 agent. Change them only through the orchestrator.

- `model::Frame`: tightly packed, top-down, premultiplied BGRA8. No native pointer or COM object crosses a thread channel.
- `model::NormRect`: `[left, top, right, bottom]` in 0..1, top-left origin, relative to the displayed page or image after edits.
- `model::TextLayer`: text in reading order plus one box per char. PDF text (`pdf.rs`) and OCR text (`ocr.rs`) both produce it. The shell builds selection, copy, search highlight, and Narrator text from it.
- `model::SearchHit`: page index plus rects.
- Edits stay recipe-based: `ImageEdit` and `PdfEdit` lists. Undo pops a recipe. Revert clears it. Every new edit kind is a new enum variant, applied to a fresh document at save time.
- PDFium calls run only on the document worker thread. Long jobs (OCR, background removal, batch export) run on a separate task worker, so they never delay page rendering.
- Existing public functions keep their signatures during wave 1. Add new functions; do not break callers in `shell.rs`.
- No network on the launch path. Models and OCR initialize on first use.
- UI text is plain, short, and sentence case. Errors say what happened and what to do.

## Wave 1: foundations (parallel)

| ID | Owner files | Deliverable |
| --- | --- | --- |
| W1-A UI foundation | `src/shell.rs` → `src/ui/**`, `src/main.rs`, `Cargo.toml` UI deps | Modular shell. Direct2D device-context renderer. Light, dark, and high-contrast themes that follow the system and switch live. Mica on Windows 11 22H2+, solid fallback on Windows 10. Custom title bar with tabs and snap-layout support. Toolbar (6 buttons, overflow at narrow widths), markup bar (Ctrl+Shift+A), sidebar frame, status. Custom themed popup menus and in-window sheets (dialogs) with EDIT children for text input. Widget layer with keyboard focus, Tab order, access keys, and tooltips. UI Automation through AccessKit. Pointer input through WM_POINTER (mouse, pen with pressure, touch, pinch zoom). Every existing feature still works. |
| W1-B PDF engine | `src/pdf.rs`, `tests/pdf_*.rs` | Page sizes; region (tile) render at any scale with annotations and form fields drawn; `TextLayer` per page in reading order; search returning all `SearchHit`s with a cancel flag (500 pages under 1 s); outline (TOC); metadata; annotation list, delete, and note edit; inline form-fill session (click, type, Tab, checkbox, radio, list) that emits `FillField` recipes; insert image as page; merge with optional form flattening; incremental save from a fresh document handle; arrows and all 9 markup kinds visible in other readers. |
| W1-C Imaging and AI | `src/imaging.rs`, `src/ocr.rs`, `src/background.rs`, their tests | Export options (format, quality 0..1) returning bytes written; size estimate before save; lossy WebP with quality; HEIC export only when Windows has an encoder; clipboard image read and write; batch with progress and cancel; OCR returning `TextLayer`; background removal meeting the 3 s target on this PC's CPU path if possible, with a model whose license allows commercial use. |
| W1-D Windows integration | `src/integration.rs` (new), `packaging/**` (new), `tools/package.ps1`, `tools/register-file-associations.ps1`, `tests/integration_*.rs` | Single-instance handoff (WM_COPYDATA); file associations and "Open with"; link to Default apps settings; Explorer verbs Convert, Resize, Combine into PDF (multi-select); Windows share sheet; drag-out source (CF_HDROP); recent files; MSIX package build and validation; winget manifest template. Not wired into the shell until wave 2. |
| W1-E Benchmarks | `tools/benchmark.ps1`, `tools/test-benchmark.ps1`, `tools/perf-gate.ps1` (new), `docs/benchmarks.md` | Harness runs on Windows PowerShell 5.1. Process-relaunch launch p95 (30+ runs), first page of the 500-page PDF, memory with a 20-page PDF, package size. Baseline JSON and a gate that fails on a regression over 10%. |

## Wave 2: v1 tasks on the new shell (PRD order, after W1-A merges)

| Slice | PRD task | Done when (PRD) |
| --- | --- | --- |
| W2-1 Viewer | 1 | Continuous scroll, single and two-page views, zoom modes, thumbnail sidebar with TOC and notes tabs, byte-budget page cache, newest-first cancelable render queue, next-image pre-decode. First page under 400 ms; 60 fps; no blank tiles after 100 ms. |
| W2-2 Text | 2, 9 | Mouse and keyboard text selection on PDFs and images (OCR on hover, offline). Ctrl+F finds text in 500 pages under 1 s, with hit highlights. Copy keeps reading order. |
| W2-3 Save model | all | Autosave like Preview. First edit asks once: overwrite or save a copy, with "Remember my choice". Revert to opened. Incremental PDF saves. Crash-safe writes. |
| W2-4 Sign, forms, markup | 3, 4, 5 | Signature drawn once with mouse, pen, or touch, saved locally, placed in 2 clicks. Inline AcroForm typing with Tab. Free text on non-form PDFs. 9 markup tools saved as standard annotations, readable in Edge and Acrobat. |
| W2-5 Organize pages | 6 | Drag to reorder, delete, rotate, insert blank or image page; drag a thumbnail out to Explorer creates a PDF; drop a file into the sidebar merges. |
| W2-6 Image edits and convert | 7, 8 | Crop by drag; resize by pixels or percent with aspect lock; file size shown before save; export to JPEG, PNG, WebP, TIFF, HEIC, PDF with quality slider; batch on 100 files in one action. New from clipboard. |
| W2-7 Background and integration | 10, also-in-v1 | One-click background removal under 3 s; copy or save as transparent PNG. Share, print, slideshow, tabs, single instance, default-app offer, Explorer verbs, empty state with recent files and "New from clipboard". |

## Wave 3: release gates

1. Accessibility and keyboard audit: UI Automation tree check, Narrator labels, high contrast, 225% text scale, a shortcut and a right-click entry for every action.
2. Performance gate on the final build; update `docs/benchmarks.md`.
3. Annotation round trip: render saved PDFs with an independent renderer and in Edge.
4. Packaging: ZIP and MSIX, third-party notices complete.
5. Final adversarial review of the whole app against the PRD.

## Known blockers needing the owner

- Reference laptop and 4 GB eMMC laptop for the PRD launch gate.
- ARM64: MSVC ARM64 tools need an admin install.
- Store and winget publishing, code-signing certificate, AI model download host.
- Lawyer review: HEVC, LGPL EULA, AI model training data (Snap: undisclosed; Depth Anything V2 Small: teacher trained on VKITTI 2, CC BY-NC-SA 3.0).

## Slice status

| Slice | Status |
| --- | --- |
| W1-A to W1-E | Merged into `v1`. Reviewed; fixes merged or in progress (W1-C fixes). |
| W2-1 Viewer | Merged into `v1`. Verifier running. |
| W2-2 Text | Implementing |
| W2-3 Save model | Implementing |
| W2-4 to W2-7 | Not started |

## Open findings (verified, not fixed yet)

From the W2-1 verifier, 2026-10-07. Failing tests: `scratchpad/review-w2-1/repo`, names start with `verifier`.

1. Major. `src/ui/app.rs:828`: `received` clears `state.sent` for every non-PDF result, so an open image re-requests its neighbors forever (16,598 cycles in 2 s; each reads and sorts the folder). Fix: clear `sent` only for stale PDF results, or remember finished pre-decodes.
2. Major. Tile cache churn at 4K and after zooming out: 48 MiB budget (`cache.rs:11`), half-screen margins (`document.rs:738`), and old-scale fallback tiles pinned by `get` every frame (`document.rs:604`). Fix: draw fallback only where a current tile is missing (use `peek`), stop when complete, and size the budget or prefetch from the viewport.
3. Minor to major. `document.rs:230` `set_top`: the end of a scroll glide makes the center page current and overrides `go_to_page`, so at 25% zoom the last page never becomes current (Delete and Rotate hit the wrong page; slideshow never ends). Fix: keep an explicit target page during a glide.
4. Minor. `actions.rs:107`: closing the sidebar leaves focus on a thumbnail, so arrow keys change pages. Fix: move focus to the document.
5. Minor. A running pre-decode cannot be interrupted, so Right arrow in the first 200 ms can miss 50 ms. Fix: pre-decode the next image first, the previous one later.
6. Simplify. `actions.rs:557` `civil()` re-implements `FileTimeToSystemTime`; `received()` deep-compares edit lists per tile (use `Arc::ptr_eq`).
