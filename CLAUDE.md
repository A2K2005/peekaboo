# Preview for Windows

A free, native Windows app that opens PDFs and images in under 400 ms and does everyday edits. The product spec is [docs/PRD.md](docs/PRD.md). The PRD is the source of truth for scope, performance targets, and stack. Do not change scope or stack without owner approval.

## Current status

Phase 1 (research) is complete. Read [docs/research/SUMMARY.md](docs/research/SUMMARY.md) first. Phase 2 waits for owner answers on Windows 10 support and HEIC in v1 (SUMMARY, "Questions for the owner"). The owner's usage budget cut research short; unmeasured items are listed under "Pending measurements" in SUMMARY.

| Phase | Status | Output |
| --- | --- | --- |
| 1. Research | Done (some measurements pending) | `docs/research/*.md`, `docs/research/SUMMARY.md` |
| 2. Architecture | Not started | `docs/architecture.md`, `docs/adr/` |
| 3. Speed spike | Not started | Shell, benchmark harness, `docs/benchmarks.md` |
| Stop for owner review | — | Owner gives go before Phase 4 |
| 4. Build v1 | Not started | 10 v1 tasks as vertical slices |

## Architecture summary

Proposed by Phase 1, final in Phase 2 ADRs: C# on WinUI 3 (Windows App SDK 2.5.1), .NET 10 NativeAOT, packaged MSIX. PDFium without V8 on one dedicated thread, through hand-written `[LibraryImport]` bindings. WIC for images, libwebp for WebP export. Windows.Media.Ocr for text in images. Background removal model and runtime download on first use.

## Build and test commands

None yet. Toolchain on the dev PC: see "Dev environment".

## Conventions

- Every claim in a research or design doc cites a source URL, a measured benchmark, or working code. Unverified claims say "unverified".
- No GPL or AGPL code or dependencies. Read GPL projects for patterns only. Never copy their code.
- No network calls, update checks, or telemetry on the launch path.
- Simple beats complex. One clear way to do each thing. No speculative abstractions.
- Fix root causes, not symptoms.
- An agent never reviews its own work.
- Prose in docs, comments, commits, and UI text: Google Developer Documentation Style Guide, ASD-STE100-derived precision, Zinsser (clarity, simplicity, brevity, humanity).
- Research docs live in `docs/research/<topic>.md`. Proof code lives in `docs/research/proofs/<topic>/`.

## Dev environment

| Item | Value |
| --- | --- |
| OS | Windows 10 Home 22H2, build 19045 |
| CPU, RAM, GPU | Intel i7-11800H, 32 GB, NVIDIA RTX 3050 Ti Laptop + Intel UHD |
| Installed | git 2.17, Python 3.14, Node.js, PowerShell 5.1, .NET Framework 4.8 `csc.exe` (C# 5), VS Build Tools 2026 (MSVC 14.50, x64 only), Windows 11 SDK 10.0.26100, Rust stable (`x86_64-pc-windows-msvc`) |
| .NET SDK 10 | Per-user install at `%LOCALAPPDATA%\Microsoft\dotnet` (no admin). Call `%LOCALAPPDATA%\Microsoft\dotnet\dotnet.exe` or put that folder first on `PATH`. |
| Missing | MSVC ARM64 tools, CMake |

This PC is not the PRD reference laptop (i5 12th gen, 8 GB, Windows 11) or the low-end check PC (4 GB, eMMC). Benchmarks from it do not prove the PRD gates.

## Decisions log

| # | Date | Decision | Reason |
| --- | --- | --- | --- |
| D1 | 2026-10-06 | Repo lives at `Desktop/extension/preview-for-windows`, its own git repo. | Owner asked for a new folder. Inside the session folder so the app's file pane can open it. |
| D2 | 2026-10-06 | Research agents may use Python venvs and official binaries in the session scratchpad for proofs. They do not install system software. | Proofs need working code. System installs need owner approval. |
| D3 | 2026-10-06 | The language and UI stack are open (owner: "Rust, Go, or whatever you want"). A seventh research agent compares stacks in `docs/research/ui-stack.md`. Scope and targets stay fixed. | Owner approval to change the stack. Choose by measured evidence, not preference. |
| D4 | 2026-10-06 | The owner leans toward Rust. Rust is a tie-breaker only when candidates are within measurement noise. A Rust core behind a native UI shell is also evaluated. | Owner asked for "best, fastest, efficient". A preference does not override hard requirements: accessibility, pen input, Fluent look, 15-week effort. |
| D5 | 2026-10-06 | Propose C# + WinUI 3 + NativeAOT over Rust. D4's tie-breaker does not apply. | Rust lost on hard requirements (UIA, IME, drag-out, pen), not within measurement noise. See `docs/research/ui-stack.md`. Final in the Phase 2 ADR. |
| D6 | 2026-10-06 | Research stopped early to save the owner's usage budget. Unrun measurements move to Phase 3. | Owner request. |
