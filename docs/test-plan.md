# Executable test plan

## Current evidence

The fixture generator ran successfully on the local Windows 10 x64 host. Two consecutive runs produced identical manifest hashes. Each PDF object's xref offset is checked by reading the generated file. This checks structure generated here; it does not replace decoding by PDFium or another PDF reader.

The benchmark harness self-test accepts a synthetic valid marker and rejects missing/invalid markers. These self-test timings measure PowerShell helper processes, not Preview. Raw evidence is under `artifacts/harness-self-test/`. The benchmark contract is in [contracts.md](contracts.md).

Independent integration tests passed against the real WIC and PDFium implementations: image workflow 4 passed; PDF workflow 4 passed and one implementation-owned test ignored. The independent workflows cover PNG pixel/alpha preservation, white compositing for JPEG, rotate/crop/resize, full 24 MP source-resolution PNG export, overwrite refusal, PDF edit ordering and text search, save/extract/merge, AcroForm text fill/reopen, protected form-merge refusal, all nine annotation types producing saved visible appearances, and unchanged source files. These are engine tests, not GUI or Edge/Acrobat compatibility tests. Generated outputs remain under `artifacts/image-workflow/` and `artifacts/pdf-workflow/`.

The initial full Cargo invocation built the integration executables but failed on unrelated in-progress shell code. Those executables were then run directly and passed. Re-run the full command after integration fixes:

```powershell
Copy-Item -LiteralPath runtime/pdfium.dll -Destination target/release/pdfium.dll
& C:/Users/Armaan/.cargo/bin/cargo.exe test --offline --release --test image_workflow --test pdf_workflow -- --test-threads=1
```

Native UI automation was stopped at the user's request. No further native UI tests were attempted. Keyboard focus, Narrator, visual UI and Edge/Acrobat annotation round trips remain unverified.

## Commands

Run from the repository root with PowerShell 7:

```powershell
pwsh -NoProfile -File tools/make-fixtures.ps1
pwsh -NoProfile -File tools/test-benchmark.ps1
pwsh -NoProfile -File tools/benchmark.ps1 -Executable target/release/preview-for-windows.exe -InputFile fixtures/20-pages.pdf -Runs 30
pwsh -NoProfile -File tools/benchmark.ps1 -Executable target/release/preview-for-windows.exe -InputFile fixtures/500-pages-50mb.pdf -Runs 30
pwsh -NoProfile -File tools/benchmark.ps1 -Executable target/release/preview-for-windows.exe -InputFile fixtures/image-24mp.jpg -Runs 30
```

Use the actual built executable path if its name changes. Paths are passed with `ProcessStartInfo.ArgumentList`, without a command shell. Each session gets a unique output directory containing raw marker files, `samples.json`, `samples.csv`, and `summary.json`. A missing success marker, nonzero exit, malformed marker, or timeout fails the run. A percentile over successful samples does not erase failed runs.

## Fixtures and limits

| Fixture | Purpose | Limit |
| --- | --- | --- |
| `20-pages.pdf` | Opening, page count, page navigation, memory | Simple built-in font and one line per page |
| `500-pages-50mb.pdf` | First visible page from a 50,145,534-byte, 500-page PDF | Whitespace-padded streams; not a realistic scan, font, transparency or vector stress test |
| `image-24mp.jpg` | 6000 by 4000 WIC image decoding | Flat synthetic pattern compresses well; no EXIF, ICC, texture or decoder worst case |
| `image-small.png` | Alpha-capable image format and basic decode | Current pattern is opaque |
| `corrupt.pdf`, `corrupt.jpg` | Honest error display; no success marker | Minimal malformed input, not a fuzz corpus |

Generation uses bounded PDF stream writes and a single image bitmap. No external PDF library or downloaded test document is required. The fixture manifest records file lengths and SHA-256 hashes.

## Measurement interpretation

- Launch starts immediately before `Process.Start`. The app emits a marker only after drawing content and successfully flushing composition. The delta includes process creation, decoding, rendering and that composition proxy. It is not independent display scan-out measurement.
- Repeated fresh processes are **process-relaunch**, not cold cache, cold boot or resident warm activation. Do not compare their p95 directly with the PRD's cold/warm gates as a pass.
- p95 uses nearest rank: sorted sample at `ceil(0.95 * n)`. Preserve every raw sample and failures.
- Memory records sampled private bytes and working set, plus the OS-reported peak working set and peak private commit through the last successful query. Samples are about 5 ms apart and scheduling can increase that interval. Memory includes launch only because the app autocloses; it does not establish steady-state, cache-pressure or long-session memory.
- The reference i5/8 GB/SSD Windows 11 and 4 GB/eMMC gates remain unverified until those devices are tested. Record device model, CPU, RAM, storage, OS build, app hash, PDFium hash, file hash, power mode and display scale in that run's evidence.
- Next-image latency, 60 fps scrolling and blank-tile duration are not inferred from launch markers. Add per-request draw markers and an external input timestamp before reporting navigation latency. Use ETW/PresentMon or independent frame capture for frame pacing. Repaint elapsed time is not FPS.

The harness does not yet automate navigation, resident activation, cache eviction, or frame pacing. These measurements remain open work, not passing targets.

## Runtime smoke and edge checks

| Scenario | Expected result |
| --- | --- |
| Valid image and PDF | Actual visible content, correct aspect ratio and page count; exactly one first-content marker |
| Resize/minimize/restore | No crash, image remains legible, no decoder work on the paint thread |
| Rapid file changes/navigation | Latest requested content wins; stale decode cannot replace it |
| Missing path or denied access | Actionable inline error; no success marker; another file can still be opened |
| Corrupt input | Error, preserved prior frame if any, no crash or success marker |
| Password PDF | Request password if implemented; otherwise explicit unsupported/password-required error. Never show blank page as success |
| XFA form | Honest unsupported-form state if not supported; never promise arbitrary Edge installation can fill it |
| HEIC without codec | Explicit unsupported-codec error in the current user-approved v2 deferral. No silent network/download or crash |
| Huge dimensions | Checked allocation limit with readable error; no integer overflow or unbounded allocation |
| Unicode/space-containing path | Open the exact supplied file; no shell interpretation |
| Keyboard/Narrator/high contrast/200% text | Visible focus, named controls, operable commands and no clipped essential actions |

Password and valid HEIC fixtures are not generated by this script. Obtain a redistributable known-password PDF and real HEIC fixture with provenance before checking those rows. A random file renamed `.heic` tests corruption, not missing-codec behavior. Real 500-page scans and vector-heavy files are also needed before a performance claim.

## Sources

- [QPC and cross-process timestamps](https://learn.microsoft.com/windows/win32/sysinfo/acquiring-high-resolution-time-stamps)
- [DwmFlush](https://learn.microsoft.com/windows/win32/api/dwmapi/nf-dwmapi-dwmflush)
- [Process memory counters](https://learn.microsoft.com/windows/win32/api/psapi/ns-psapi-process_memory_counters_ex)
- [ArgumentList](https://learn.microsoft.com/dotnet/api/system.diagnostics.processstartinfo.argumentlist)
- [Accessibility checks](https://learn.microsoft.com/windows/apps/design/accessibility/accessibility-checklist)

Implementation review must be performed by another agent. Passing these scripts does not constitute independent review.
