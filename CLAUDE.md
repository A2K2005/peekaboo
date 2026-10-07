# Preview for Windows

A free, native Windows app that opens PDFs and images in under 400 ms and does everyday edits. The product spec is [docs/PRD.md](docs/PRD.md). The PRD is the source of truth for scope and performance targets. The owner opened the language and stack choice (D3). Do not change scope without owner approval.

## Current status

Building v1 end to end. Plan and slice status: [docs/plan-v1.md](docs/plan-v1.md). Per-task acceptance: [docs/acceptance.md](docs/acceptance.md). Current handoff and remaining work: [docs/remaining-work.md](docs/remaining-work.md).

| Phase | Status | Output |
| --- | --- | --- |
| 1. Research | Done (some measurements pending) | `docs/research/*.md`, `docs/research/SUMMARY.md` |
| Codex development build | Merged here (D9) | Rust app: viewing, page edits, markup, images, OCR, background removal |
| Wave 1: foundations | Implemented; current worktree contains reviewed fixes | UI foundation, PDF engine, imaging and AI, Windows integration, benchmarks |
| Wave 2: v1 tasks | In progress; viewer and save model reviewed, text integrated with final fixes awaiting rerun, later workflows partial | 7 slices in PRD order |

Branches: `master` holds the research and the Codex merge. All wave work merges into `v1`. Codex's checkpoint `8c0ca65` (branch `codex/glance-gap-fixes`, worktree `Desktop/Portfolio/preview-for-windows-latest`) is fast-forwarded into `v1`. Remote: private repo `A2K2005/preview-for-windows` on GitHub (D20). Nothing merges into `master` without the owner (D14).
| Wave 3: release gates | Not started | Accessibility, performance, round trip, packaging, final review |

## Architecture summary

Rust 2021 with windows-rs 0.62 on Win32 and Direct2D. One UI thread. One document worker owns every PDFium and WIC viewing call. A task worker runs OCR, background removal, and batch jobs. Results carry generation IDs, so stale results are dropped. Edits are recipes (`model::ImageEdit`, `model::PdfEdit`) applied to a fresh document at save time. Shared types and threading rules: [docs/plan-v1.md](docs/plan-v1.md) "Shared contracts". Details: [docs/architecture.md](docs/architecture.md).

| Module | Role |
| --- | --- |
| `src/ui/` | Shell: `window` (message loop, title bar, input, theme and DPI changes), `app` (state, tabs, scheduling), `worker` (document worker and task worker), `commands` (command table, shortcuts, menus), `actions`, `widgets` (layout, hit test, focus, access keys), `paint`, `document` (view, pointer, pinch), `render` (Direct2D HWND target), `theme`, `menu` (custom popup menus), `sheet` (in-window dialogs with EDIT input), `a11y` (AccessKit), `files` |
| `src/pdf.rs`, `src/pdf/` | PDFium: rendering and tiles, text layer and search, forms session, annotations, page edits, incremental and full saves |
| `src/imaging.rs` | WIC decode, edits, export with format and quality, size estimate, clipboard, batch |
| `src/ocr.rs` | Windows.Media.Ocr returning `TextLayer` |
| `src/background.rs` | ONNX Runtime background removal (Snap model, CPU), loaded on first use |
| `src/integration.rs` | Single-instance handoff, file associations, Explorer verbs, share, drag out, recent files (wired in wave 2) |
| `src/printing.rs` | Print dialog, rasterizing, spooling |
| `src/model.rs` | Shared types |

## Build and test commands

Cargo: `C:\Users\Armaan\.cargo\bin\cargo.exe`.

| Task | Command |
| --- | --- |
| Fetch PDFium into `runtime/` | `powershell -File tools/fetch-pdfium.ps1` |
| Make test fixtures in `fixtures/` | `powershell -File tools/make-fixtures.ps1` |
| Build | `cargo build --release` |
| Test | `cargo test --release -- --test-threads=1` (PDFium is process-global, so tests run serially) |
| Optional real OCR test | `cargo test --release --test ocr_smoke -- --ignored --nocapture` |
| Package ZIP | `powershell -File tools/package.ps1` |

`runtime/` (PDFium, ONNX Runtime, model) and `fixtures/` are not in git. In a git worktree, link them to the main checkout: `cmd /c mklink /J runtime C:\Users\Armaan\Desktop\extension\preview-for-windows\runtime` (same for `fixtures`).

## Conventions

- Every claim in a doc cites a source URL, a measured benchmark, or working code. Unverified claims say "unverified". Never invent benchmark results or API support.
- No GPL or AGPL code or dependencies. Read GPL projects for patterns only.
- No network calls, update checks, or telemetry on the launch path. OCR and AI load on first use.
- Simple beats complex. One clear way to do each thing. No speculative abstractions.
- Fix root causes, not symptoms.
- Comments explain only a non-obvious why (a platform quirk, a safety rule, a known limit). No comments that restate the code, narrate changes, name waves, slices, or agents, or hold TODO placeholders. No dead code or unused imports.
- An agent never reviews its own work.
- Never overwrite a user's file without the save model's consent rules (W2-3). Writes go to a temp file, then an atomic replace.
- Prose in docs, comments, commits, and UI text: Google Developer Documentation Style Guide, ASD-STE100-derived precision, Zinsser (clarity, simplicity, brevity, humanity). No em dashes in UI text.
- Research docs live in `docs/research/`. Proof code lives in `docs/research/proofs/<topic>/`.

## Dev environment

| Item | Value |
| --- | --- |
| OS | Windows 10 Home 22H2, build 19045 |
| CPU, RAM, GPU | Intel i7-11800H, 32 GB, NVIDIA RTX 3050 Ti Laptop + Intel UHD |
| Installed | git 2.17, Python 3.14, Node.js, PowerShell 5.1 (no PowerShell 7), VS Build Tools 2026 (MSVC 14.50, x64 only), Windows 11 SDK 10.0.26100, Rust stable (cargo 1.99) |
| .NET SDK 10 | Per-user at `%LOCALAPPDATA%\Microsoft\dotnet` |
| Missing | MSVC ARM64 tools, CMake, Adobe Acrobat Reader |

This PC is not the PRD reference laptop (i5 12th gen, 8 GB, Windows 11) or the low-end check PC (4 GB, eMMC). Benchmarks from it do not prove the PRD gates.

## Decisions log

| # | Date | Decision | Reason |
| --- | --- | --- | --- |
| D1 | 2026-10-06 | Repo lives at `Desktop/extension/preview-for-windows`, its own git repo. | Owner asked for a new folder. Inside the session folder so the app's file pane can open it. |
| D2 | 2026-10-06 | Agents may use Python venvs and official binaries in the session scratchpad for proofs. They do not install system software. | Proofs need working code. System installs need owner approval. |
| D3 | 2026-10-06 | The language and UI stack are open (owner: "Rust, Go, or whatever you want"). | Owner approval to change the stack. |
| D4 | 2026-10-06 | The owner leans toward Rust. Rust is a tie-breaker only when candidates are within measurement noise. | A preference does not override hard requirements. |
| D5 | 2026-10-06 | Research proposed C# + WinUI 3 + NativeAOT over a from-scratch Rust build. | Rust lost on UI effort (UIA, IME, drag-out, pen), not on speed. See `docs/research/ui-stack.md`. Superseded by D9. |
| D6 | 2026-10-06 | Research stopped early to save the owner's usage budget. Unrun measurements move to later waves. | Owner request. |
| D7 | 2026-10-06 | HEIC: no bundled decoder. Use the Windows codec when installed. | Owner: skip HEIC if it blocks. Patent and LGPL risk (`docs/research/licensing.md`). |
| D8 | 2026-10-06 | A parallel Codex build existed at `Desktop/Portfolio/preview-for-windows` (Rust, Win32, Direct2D). It built, and 23 tests passed. | Checked before deciding which codebase continues. |
| D9 | 2026-10-06 | Continue the Codex Rust build, merged into this repo. The Portfolio copy stays untouched. | Owner: "do everything end to end; build missing features properly". Working, tested code removes much of Rust's effort penalty from D5. |
| D10 | 2026-10-06 | Support Windows 10 22H2 and Windows 11, as the PRD says. Mica on Windows 11 22H2+, solid backdrop elsewhere. | The Win32 stack has no WinUI-on-Windows-10 launch penalty, so the owner question from Phase 1 no longer changes a decision. |
| D11 | 2026-10-06 | UI Automation through AccessKit (MIT/Apache-2.0) over hand-written UIA providers. Text input uses Win32 EDIT child controls for IME. Menus and dialogs are custom-drawn, so they follow dark mode without undocumented APIs. | Reuse a maintained UIA layer; Win32 menus have no documented dark mode. |
| D12 | 2026-10-06 | One verifier agent per slice does both edge-case testing and review. | Keeps "never review your own work" at lower usage cost. |
| D13 | 2026-10-06 | Fixed `pdfium.dll` lookup for test binaries in `target/<profile>/deps`. | Codex's tests passed only because a stray DLL copy existed in its build folder. |
| D14 | 2026-10-07 | Git: commit only when the owner asks. Exception: wave agents commit to their own local worktree branches. The orchestrator reports to the owner before any merge or commit on `master`. Nothing is pushed. | Owner objected to unrequested commits, then allowed branch commits for parallel work. |
| D16 | 2026-10-07 | Background removal ships the Snap model (Apache-2.0 weights, undisclosed training data) on the CPU path: 0.31 s for 12 MP, IoU 0.90 against BiRefNet. BiRefNet, BEN2, and ormbg are rejected: trained on data with no-commercial-use terms. DirectML stays off: the iGPU was slower than the CPU and the NVIDIA path hit GPU timeouts. | Measured in W1-C. Lawyer review before release: Snap's training data is undisclosed, and its Depth Anything V2 Small backbone (Apache-2.0 weights) was distilled from a teacher trained on VKITTI 2 (CC BY-NC-SA 3.0) and pseudo-labeled research-only sets (W1-C review). |
| D17 | 2026-10-07 | Arrows save as Ink annotations. | PDFium cannot create Line annotations. Ink has an appearance stream; pdf.js draws it (W1-B test). Edge and Acrobat display is unverified. |
| D15 | 2026-10-07 | No GUI launches while the owner uses the PC. Headless tests only. GUI checks (screenshots, UIA dumps, launch timing) run only after the owner says they are away. | Owner revoked an earlier "launch briefly" permission after test windows disrupted their work. |
| D18 | 2026-10-07 | The visual direction is a restrained dark glass Windows shell: Mica commanding/content layers, near-black opaque viewer canvas, subtle edge halos, sparse system accent, opaque transients, and solid/high-contrast fallbacks. | Owner supplied dark glass references. The app adapts their material and depth without copying their dashboard or onboarding composition, and without copying Glance. |
| D20 | 2026-10-07 | The repo is pushed to the private GitHub repo `A2K2005/preview-for-windows`, all branches. Every commit is authored as A2K2005 (history rewritten once; backup bundle in `%TEMP%\pfw-before-author-rewrite.bundle`). Push only when the owner asks. | Owner asked for the repo under the A2K2005 account. Supersedes "Nothing is pushed" in D14. |
| D21 | 2026-10-07 | The product becomes Quick Look first: press Space on a file in Explorer and a minimal peek window opens at once; the full editor opens only on request. The editor's chrome shows on demand. Spec: `docs/quicklook-spec.md`. | Owner: "it should be like Quick Look / Peek... very seamless... it should not act as a buffer between the user and what they want to open." |
| D22 | 2026-10-07 | Saving is Mac style: edits autosave into the file with no first-edit prompt. "Revert to opened" is the way back. Unwritable files ask where to save a copy. The product name stays for now; pick a final name before Store release. | Owner chose both. Supersedes the first-edit consent in D19 and W2-3. |
| D19 | 2026-10-07 | Autosave replays edits from one immutable opened snapshot and identifies work by the logical file path. Every overwrite requires first-edit consent; external changes pause; reader leases block snapshot cleanup. | Prevents double-applied edits, stale completions, cross-tab status errors, and close/logoff data loss found during adversarial review. |
