# Remaining work

Updated 2026-10-07 for the handoff commit. The PRD remains the release contract. This file separates implemented code from work that still needs validation or completion.

## Current checkpoint

- Viewer, cache, image predecode, Windows integration, dark glass material system, and the safe-save workflow are implemented and independently reviewed.
- Before the latest text fixes, the complete serial release suite exited successfully. The binary suite reported 150 passed and 2 optional tests ignored; the remaining integration suites also passed.
- PDF/image text selection, reading-order copy, Find, OCR hover, highlights, and accessibility text were then integrated. The first pass passed 129 UI tests with 1 timing probe ignored.
- Independent text review found four issues: raw fallback copy order, permanent text-layer retry loops, missing snapshot leases for queued view work, and word selection at the last glyph. Fixes and regressions are present in the worktree, including the multi-frame image Ctrl+A correction, but the owner asked to stop testing before the final rerun. Treat the latest text changes as **implemented but unverified**.
- 2026-10-07: the full serial release suite passed at `8c0ca65` (280 passed, 0 failed, 14 ignored). The four text fixes have no independent review yet.

## Product work left

1. **Verify and finish Find and Copy**
   - Compile and run the focused text, UI, PDF permission, and snapshot-reader tests after the final review fixes.
   - Re-review the four fixes above.
   - Replace the modal Find prompt with a persistent inline find bar if the PRD interaction target requires it.
   - Run live pointer, keyboard, Narrator, rotated-page, multi-column PDF, screenshot, TIFF, and GIF checks.

2. **Finish forms, signing, and markup**
   - Add inline AcroForm typing and complete Tab order for text, checkbox, radio, and list fields.
   - Validate signature placement with mouse, pen, and touch.
   - Verify every standard annotation and appearance in Edge and Adobe Acrobat Reader.
   - Add selection-based highlighting and a visible retry path for failed saves.

3. **Finish page organization**
   - Add thumbnail drag reorder, insertion feedback, image-page insertion UI, file drop into the sidebar, and drag-out to Explorer.
   - Validate form-preserving merge/import behavior and large-document operations.

4. **Finish image workflows**
   - Add percent resize, explicit aspect lock, quality controls, and encoded size estimates before save.
   - Complete the batch UI and run the 100-file usability/performance case.
   - Complete selectable OCR words on images and transparent copy/save after background removal.
   - Keep HEIC conditional on the installed Windows codec until licensing and distribution are approved.

5. **Finish Windows product integration**
   - Add the first-run default-app offer and recheck default status on activation.
   - Run real Explorer registration, multi-select verbs, uninstall, and reinstall tests.
   - Finish signed MSIX, Store, and winget delivery.

6. **Complete release evidence**
   - Run live Windows 11 Mica checks, Windows 10/transparency-off fallback, native field focus, 150% DPI, and 225% text interaction.
   - Record cold-launch p95, warm handoff, first page of a 500-page PDF, next-image latency, scroll frame rate, private bytes, and working set on the PRD reference and low-end PCs.
   - Run keyboard-only, Narrator, high-contrast, pen, and touch acceptance.
   - Run cross-reader, corrupt-file, password-PDF, XFA, huge-image, and missing-codec corpora.
   - Resolve packaging license blockers. `libwebp-sys` and AccessKit published crates do not provide every notice file required by the current fail-closed packaging policy.
   - Add crash-abandoned snapshot cleanup or recovery and test forced termination.

## Known limits

- No live GUI was launched during this checkpoint because the owner asked not to interrupt the desktop.
- The dark glass screenshots prove headless colors and layout, not live DWM Mica composition.
- No current measurement proves that Preview for Windows is faster than Glance or meets the PRD cold-launch and memory targets.
- Packaging intentionally fails closed while dependency notices are incomplete.
- The handoff commit is a development checkpoint, not a release candidate.

