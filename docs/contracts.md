# Speed spike contracts

Status: implementation contract, 2026-10-06. The owner delegated language choice and asked to prioritize building after providing previous research. This spike uses Rust, windows-rs 0.62.2, Win32, Direct2D, WIC, and dynamically loaded PDFium. Scope and performance thresholds remain unchanged. This is a testable native path, not a claim that Rust outperforms NativeAOT.

## Ownership

- Shell agent: `src/main.rs`, `src/shell.rs` if needed. Own window, file picker, drag/drop, navigation, presentation, async scheduling, benchmark markers.
- PDF agent: `src/pdf.rs`, `tools/fetch-pdfium.ps1`, PDF runtime provenance. Own PDFium loading, lifetime, serialized rendering.
- Test agent: `tools/benchmark.ps1`, `tools/make-fixtures.ps1`, external smoke tests, `docs/test-plan.md`. Must not review its own harness.
- Coordinator: Cargo manifest, `src/model.rs`, `src/imaging.rs`, architecture, ADRs, repository guidance, integration fixes.

## Shared types

`model::Frame`: `width: u32`, `height: u32`, `pixels: Vec<u8>`, `page_count: u32`, `source_width: u32`, `source_height: u32`.

Pixels are tightly packed, top-down, premultiplied BGRA8. Stride is `width * 4`. No native pointers or COM objects cross a channel. Width and height must be nonzero; buffer length must equal width times height times four. Target image allocations are capped at 64 MiB. Decoder source dimensions are checked before expensive work.

`imaging::decode(path: &Path, max_width: u32, max_height: u32) -> Result<Frame, String>` runs on a COM-initialized worker. WIC handles remain on that worker.

`pdf::PdfEngine::new() -> Result<PdfEngine, String>` loads PDFium only from an absolute path under the executable directory (`pdfium.dll`) or project `runtime/pdfium.dll` resolved relative to executable, never current directory search. `render(&mut self, path: &Path, page_index: u32, max_width: u32, max_height: u32) -> Result<Frame, String>`. One engine on one worker owns every PDFium call. A retained document may accelerate next-page rendering. Password/corrupt/unsupported input returns actionable errors. No save API in the spike.

## Shell

Optional first CLI argument is a file path. Open via Ctrl+O, native file menu/picker, and Explorer drop. Page navigation uses PageUp/PageDown or arrows; image navigation uses sibling paths in sorted order. Paint a window before dispatching decode. Decode and PDF rendering never run in WM_PAINT. Requests have generation IDs; ignore results from earlier generations. Keep the last good frame visible while work runs. Display errors without blocking dialogs.

## Measurement

QPC provides timestamps across processes. If `PFW_BENCH_OUT` is present, write one JSON record after successful first content draw plus successful DwmFlush: `first_content_qpc` (i64), `qpc_frequency` (i64), `width`, `height`, `page_count`. Never emit a success marker for blank window or failed render. This is a composition-flush proxy, not independent proof of display scan-out. No normal-launch file writes for metrics.

`PFW_BENCH_AUTOCLOSE=1` closes after the marker. The harness records a QPC immediately before launching, has a timeout and checks process exit. Repeated process launches are named `process-relaunch`, never cold boot or resident warm. Raw samples are retained, p95 uses nearest rank, memory records private bytes and working set separately. Reference-device gates remain unverified on this machine.

## Sources

- https://github.com/microsoft/windows-rs
- https://learn.microsoft.com/windows/win32/wic/-wic-about-windows-imaging-codec
- https://pdfium.googlesource.com/pdfium/+/refs/heads/main/public/fpdfview.h
- https://learn.microsoft.com/windows/win32/sysinfo/acquiring-high-resolution-time-stamps
- https://learn.microsoft.com/windows/win32/api/dwmapi/nf-dwmapi-dwmflush
