# Verification

Date: 2026-10-06. Host: Windows 10 22H2, build 19045; Intel i7-11800H. This is not the PRD's Windows 11 i5/8 GB reference or 4 GB eMMC device. Do not translate these measurements into reference-device passes.

## Automated evidence

The final integrated optimized test run passed 23 test executions, with shared helper tests repeated across test binaries. The optional PDF-runtime test also passed separately after final integration; OCR and AI tests passed separately. Independent image/PDF workflow tests verify real saved output, source preservation, and collision refusal. See `tests/`, `docs/runtime-review.md`, and generated evidence under `artifacts/`.

- OCR recognized `Preview for Windows Offline text recognition 12345` without network. One later local run took 89.65 ms on a generated 1200 × 260 image. This is not a multilingual or photo accuracy benchmark.
- Background removal produced a 4000 × 3000 transparent PNG in 19.1 seconds, including setup and export. The automated test found both transparent background and retained foreground. Visual inspection found a green fringe around parts of the dog's fur. Quality parity with Photos is not established.
- A DirectML probe failed with `DXGI_ERROR_DEVICE_HUNG`. The app uses the verified CPU path. A CPU model-only probe took 17.36 seconds.
- Actual AES-256 PDF fixtures exercise missing, wrong, correct, and restricted passwords. Permission checks and protected-output refusal have targeted test evidence.

## Launch measurements

An earlier viewer build, before editing/OCR/UI integration, was run five times per file through `tools/benchmark.ps1`. These are process-relaunch measurements with cached OS state, not cold boot or resident warm activation.

| Input | Earlier-build p95 |
| --- | ---: |
| Synthetic 20-page PDF | 1273.0 ms |
| Synthetic 500-page, 50.15 MB PDF | 407.8 ms |
| Generated 24 MP JPEG | 413.8 ms |

The PDF's size padding does not emulate a complex production document. The first 20-page run incurred additional cold effects. A 120 ms initial debounce was subsequently removed. **These numbers do not describe the final build.** New desktop runs were not performed after the user stopped desktop control. Cold, resident warm, scrolling, neighbor-image latency, steady-state memory, and regression gates remain unverified.

The first-content marker uses QPC after a successful document draw and DWM flush. It is a composition-flush proxy, not independent display scan-out proof. Raw earlier samples are under `artifacts/benchmarks/`.

Sources: [QPC](https://learn.microsoft.com/windows/win32/sysinfo/acquiring-high-resolution-time-stamps), [DwmFlush](https://learn.microsoft.com/windows/win32/api/dwmapi/nf-dwmapi-dwmflush), [BiRefNet model preprocessing](https://huggingface.co/ZhengPeng7/BiRefNet_lite).
