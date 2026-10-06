# Research summary (Phase 1)

Date: 2026-10-06. Author: orchestrator. Status: complete; some measurements are pending.

This file combines seven research reports. Each finding links to the report that holds its sources and proofs. The owner's usage budget cut the research short. Each report lists what it could not check under "Open questions".

| Report | Topic | Evidence type |
| --- | --- | --- |
| [reference-architecture.md](reference-architecture.md) | PowerToys Peek, Files, SumatraPDF, ImageGlass, QuickLook | Source code, issues |
| [ux-teardown.md](ux-teardown.md) | Preview, Photos, Edge, PDFgear; Fluent; our interaction spec | Vendor docs (124 sources) |
| [pdf-engine.md](pdf-engine.md) | PDFium, qpdf | Working proofs in `proofs/pdfium/` |
| [winui-performance.md](winui-performance.md) | WinUI 3 launch path | Published data; harness in `proofs/winui/` |
| [imaging-ai.md](imaging-ai.md) | WIC, HEIC, OCR, background removal | Codec probes in `proofs/imaging/` |
| [licensing.md](licensing.md) | Licenses, patents, Store terms | License files, patent pool sites |
| [ui-stack.md](ui-stack.md) | 16 language and UI stacks | Scored table; Rust hello window measured |

## Key findings

1. **Stack: keep C# on WinUI 3 with NativeAOT.** It scored 34 of 39, the highest of 16 stacks. It is the only mature option with Fluent controls, UI Automation, IME, pinch zoom, and shell integration built in ([ui-stack.md](ui-stack.md)). Current versions: Windows App SDK 2.5.1, .NET 10 LTS, CsWinRT 2.3.1, Win2D 1.4.0 ([winui-performance.md](winui-performance.md)).
2. **Rust loses on hard requirements, not on speed.** A Rust Win32 + Direct2D hello window measured 170.6 ms median and 223.4 ms p95 (10 warm runs, dev PC). But raw Win32 needs every control, UIA provider, and IME path built by hand. Microsoft's Rust WinUI 3 projection is a preview and crashes on Windows 10. Slint, iced, and gpui lack drag-out to Explorer, pen pressure, or Narrator support ([ui-stack.md](ui-stack.md)).
3. **PDFium covers 15 of 18 v1 PDF tasks, proved by code.** Open plus page 1 of a 500-page, 52 MB PDF took 10 ms. A 500-page search took 0.86 s. PDFium cannot create Line (arrow), Polygon, Polyline, Caret, or Redact annotations. Page merge drops form fields. Copied text follows content-stream order, not visual reading order ([pdf-engine.md](pdf-engine.md)).
4. **PDFium is not thread-safe, even across documents.** QuickLook puts one global lock around every call. SumatraPDF renders newest request first, cancels pages that scroll away, and renders at most 4 pages ahead ([pdf-engine.md](pdf-engine.md), [reference-architecture.md](reference-architecture.md)).
5. **HEIC is the largest legal risk.** Access Advance now runs the HEVC pool and says downloaded HEVC software "in general" needs a license; its licensors sued HP, ASUS, Roku, and Snap. On this PC, HEIC decode fails without the HEVC Video Extension; the HEIF extension alone is not enough ([licensing.md](licensing.md), [imaging-ai.md](imaging-ai.md)).

## Conflicts between agents

| # | Conflict | Resolution |
| --- | --- | --- |
| C1 | The imaging agent cleared BiRefNet lite and BEN2 (MIT weights). The licensing agent found both trained on DIS5K, whose terms ban commercial use. | Treat both as blocked until a lawyer clears them. ormbg (Apache-2.0) is the lead candidate; check its training data first. On Copilot+ PCs, use Windows AI ImageForegroundExtractor. |
| C2 | The PDF agent said "move qpdf to v2". The same report found that PDFium page merge drops form fields. | qpdf's page merge keeps AcroForm fields (unverified; prove it in Phase 3). If it does, qpdf ships in v1 for merge. |
| C3 | Launch fallback. The PRD names C++/WinRT. The WinUI agent estimates C++/WinRT saves only 10 to 40 ms and proposes a Win32 + Direct2D first frame. The stack agent names C++/WinRT + WinUI 3 as the stack fallback. | Two different fallbacks. Stack fallback: C++/WinRT + WinUI 3. Launch fallback: a Win32 + Direct2D first frame. Decide in Phase 2 ADRs. |
| C4 | The PRD wants thumbnails on a background thread. PDFium allows one thread at a time. | One PDFium worker thread with a priority queue: visible pages first, thumbnails last. A second process for thumbnails is the option if the queue misses 60 fps. Phase 2 decides. |
| C5 | PDFium license. The PRD says Apache 2.0. The PDF agent says BSD-3-Clause. The licensing agent found both texts in the LICENSE file, plus about 20 bundled notices. | Both are permissive. Ship the full LICENSE file and all bundled notices. No change to the plan. |

## Conflicts with the PRD

| # | PRD statement | Evidence | Proposed change |
| --- | --- | --- | --- |
| P1 | Target Windows 10 and 11. | Windows 10 support ended 2025-10-14. .NET 10 does not list it. WinUI 3 measured 2.6 times slower on Windows 10 in a third-party test (813 vs 308 ms). No Mica, no Segoe UI Variable, no Segoe Fluent Icons on Windows 10. | **Owner decision.** Recommend Windows 11 only. |
| P2 | XFA forms: show "Open in Edge". | Edge does not support XFA. | Show "This form needs Adobe Acrobat Reader". |
| P3 | "Never rewrite a 50 MB PDF to add one highlight." | After rendering all 500 pages, one highlight added 52 MB. A save from a fresh document handle added 109 KB in 104 ms. | Save from a second document handle. No PRD change. |
| P4 | WIC handles WebP export and uses the HEIC codec when present. | WIC has no WebP encoder. HEIC needs the HEVC decoder, not only the HEIF extension. HEIC export needs an HEVC encoder the PRD does not name. | Bundle libwebp (BSD). Detect the HEVC decoder through Media Foundation. HEIC export only when Windows has the encoder. |
| P5 | Bundled HEIC decoder in v1. | Patent risk (finding 5). LGPL-3.0 conflicts with the Store's default terms, so we need our own EULA. | Use the PRD's own mitigation: prompt for the Windows codec. Bundle a decoder only after legal review. |
| P6 | Installer under 30 MB. | ONNX Runtime with DirectML is 39.6 MB uncompressed. ImageGlass NativeAOT ships at 33.7 to 44.3 MB. PDFium without V8 is 3.9 MB compressed. | ONNX Runtime joins the model in the first-use download. Track installer size on every build. |
| P7 | 94 ms WinUI 3 Run dialog as the launch reference. | It is a telemetry median from Insider builds. It is not a p95 cold-launch-to-first-page number. None of the five reference apps publishes a cold-launch number. | Keep the 400 ms target. Treat it as unproven until Phase 3. |
| P8 | Accessibility at 200% text scale; 6 toolbar buttons. | Windows text scaling goes to 225%. Six buttons do not fit at narrow widths; four do. | Test at 225%. Collapse toolbar buttons into an overflow menu at narrow widths. |
| P9 | Explorer "Open with" on multi-select. | Each selected file starts its own process. Explorer hides static verbs above 15 or 100 files. | Single-instance redirection. A COM `IExplorerCommand` for batch verbs. |

## Decisions these findings imply

1. Stack: C# on WinUI 3, NativeAOT, Windows App SDK 2.5.1, .NET 10. The owner's Rust tie-breaker (D4) does not apply, because Rust lost on hard requirements, not within measurement noise.
2. One dedicated PDFium thread with a newest-first, cancelable render queue and a byte-budget bitmap cache (needed for the 120 MB memory target).
3. PDFium without V8 (no XFA rendering). Hand-written `[LibraryImport]` bindings for the API subset we use.
4. Always generate appearance streams for annotations we create. Draw arrows as a standard type PDFium can write, or write the Line dictionary ourselves; Phase 2 picks one with a proof.
5. Packaged MSIX, NativeAOT, a code-built first window, and Win2D and chrome loaded after the first frame. Single-instance with a warm process for later opens.
6. A CI performance gate needs 59 or more runs per build on a physical laptop to bound p95. None of the reference projects has such a gate.
7. Run UI tests on the NativeAOT build. Files and PowerToys found bugs that appear only in AOT builds.

## Pending measurements (move to Phase 3)

| Measurement | Why it matters | Status |
| --- | --- | --- |
| C# WinUI 3 NativeAOT hello window, same harness as the Rust one | Confirms the stack choice with a number | Build failed: `vswhere.exe` not on `PATH`. Fix is in [ui-stack.md](ui-stack.md). |
| Cold launch p95 on the reference laptop and the 4 GB eMMC laptop | The PRD gate | Needs that hardware and admin rights for standby-list flush |
| WIC decode, 24 MP JPEG, full size and screen size | Next image under 50 ms | Test image ready in scratch folder |
| Windows.Media.Ocr speed and accuracy | Task 9 | Not run |
| Background removal speed and quality, 12 MP, CPU and DirectML | Task 10, under 3 s | Models and 12 test photos ready in scratch folder |
| qpdf merge keeps form fields | C2 | Not run |
| Annotation display in Acrobat Reader and Edge | Task 5 acceptance | Not run |

## Questions for the owner

1. **Windows 10 support (P1).** Recommend Windows 11 only. It removes the slowest launch platform and the Mica and font gaps. Cost: users still on Windows 10.
2. **HEIC in v1 (P5).** Recommend the Windows codec prompt in v1 and a bundled decoder only after legal review.
