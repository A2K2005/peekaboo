# UI stack comparison for Preview for Windows

Date: 2026-10-06. Author: Research agent: UI stack. Status: draft.

## Summary

- Keep the PRD stack: C# + WinUI 3 (Windows App SDK 2.5.1) + NativeAOT. It scores 34 of 39, the highest of 16 stacks. It is the only mature stack that ships Fluent controls, UI Automation, IME, pinch zoom and shell integration ready to use, and it can be built in 15 weeks [4][14][20].
- Fallback: C++/WinRT + WinUI 3 (31 of 39). It keeps the same controls and accessibility and removes the .NET runtime. It is mature: C++/WinRT 3.0 shipped in May 2026 [24].
- Rust does not win. Raw Win32 + Direct2D in Rust starts fast: 171 ms median warm on the dev PC [1]. But every control, plus UI Automation, IME and the Fluent look, must be built by hand (estimate). Microsoft's Rust WinUI 3 library (windows-reactor) is still a preview and crashes on Windows 10 when framework-dependent [33][35]. Rust toolkits fail on pen input, drag-out or screen readers [40][41][42][43]. The owner's Rust tie-breaker does not apply: the best Rust option scores 25, 9 points below WinUI.
- The deciding number is unmeasured: cold launch p95 of a WinUI 3 window on the 4 GB eMMC laptop. The C# WinUI 3 hello window failed to build here because of a toolchain PATH problem. It is pending (see "Pending measurements").
- PRD risks: Windows 10 has been out of support since 2025-10-14 [27]. InkCanvas is in experimental WinUI builds only [4]. The 94 ms Run dialog number is a median on unstated hardware, not a p95 [2].

## Findings

### Scored comparison

Scores: 0 = fails the requirement, 1 = major gaps or high effort, 2 = works with moderate work, 3 = strong out of the box. Weights: launch ×3 ("speed is the product"), accessibility ×2 and effort ×2 (hard requirement and fixed timeline), all others ×1. Maximum is 39. The cell evidence is in the sections below the table.

| # | Stack | Launch ×3 | Size, memory | Native look | Accessibility ×2 | Pen, touch, ink | Text, IME, shell | Interop | ARM64, packaging, maturity, license | Effort ×2 | Total |
|---|---|---|---|---|---|---|---|---|---|---|---|
| A | C# + WinUI 3 + NativeAOT | 2 | 2 | 3 | 3 | 2 | 3 | 3 | 3 | 3 | **34** |
| B | C++/WinRT + WinUI 3 | 2 | 2 | 3 | 3 | 2 | 3 | 3 | 2 | 2 | **31** |
| C | Hybrid: Rust core (C ABI) + C# WinUI 3 shell | 2 | 2 | 3 | 3 | 2 | 3 | 2 | 2 | 2 | **30** |
| D | WPF (.NET 10, ReadyToRun) | 1 | 1 | 2 | 3 | 3 | 3 | 3 | 2 | 3 | 29, disqualified (JIT) |
| E | Qt 6.12 (C++) | 2 | 1 | 2 | 2 | 3 | 3 | 3 | 1 | 2 | **27** |
| F | C++ + Win32 + Direct2D/DirectComposition, custom controls | 3 | 3 | 1 | 1 | 3 | 2 | 3 | 3 | 0 | **26** |
| G | Rust + windows-rs Win32 + Direct2D, custom controls | 3 | 3 | 1 | 1 | 3 | 2 | 3 | 2 | 0 | **25** |
| H | Rust + WinUI 3 (windows-reactor preview) | 2 | 2 | 3 | 3 | 1 | 2 | 2 | 1 | 1 | **25** |
| I | Rust + egui | 2 | 2 | 0 | 2 | 1 | 1 | 2 | 2 | 1 | **20** |
| J | Avalonia 12 + NativeAOT | 1 | 1 | 2 | 1 | 2 | 1 | 3 | 2 | 2 | **20** |
| K | Rust + Slint 1.18 | 2 | 2 | 1 | 2 | 0 | 1 | 2 | 1 | 1 | **19** |
| L | Rust + gpui | 2 | 2 | 0 | 1 | 0 | 1 | 2 | 1 | 1 | **16** |
| M | Rust + iced 0.14 | 2 | 2 | 0 | 0 | 0 | 1 | 2 | 2 | 1 | **15** |
| N | Rust + Xilem/Masonry 0.4 | 2 | 2 | 0 | 1 | 1 | 1 | 2 | 0 | 0 | **14** |
| O | Flutter 3.47 | 0 | 1 | 1 | 1 | 1 | 1 | 1 | 2 | 2 | **13** |
| P | Go (Fyne, Gio, walk) | 2 | 2 | 0 | 0 | 0 | 0 | 0 | 2 | 1 | **12** |

Launch scores of 2 for rows A, B, C, H, E and the Rust toolkits are estimates. No stack has a published cold p95 on hardware like the PRD laptops. Only row G is measured here [1].

Dropped without scoring:

- Electron, Tauri, Wails: each renders the UI in a browser engine (Chromium or WebView2) [73][74][75]. The PRD rejects this.
- Uno Platform: its Windows target is plain WinUI 3 ("the application isn't using Uno Platform at all") [58]. It adds nothing for a Windows-only app.
- .NET MAUI: built on WinUI 3, and Windows NativeAOT is a backlog item [57].
- WinForms: no trimming or NativeAOT (issue open, milestone "Future") [59], and no Fluent look.
- React Native for Windows: a JavaScript runtime (Hermes) and bundle load sit on the launch path [79]. No startup data exists (unverified).
- Delphi and Lazarus: paid or niche toolchains, and their accessibility depth is unknown (unverified).

### 1. Launch time

- Microsoft's new Run dialog is "a C#/WinUI 3 application compiled with dotnet AOT". It has a 94 ms median time-to-show, against 103 ms for the old Win32 dialog. Microsoft gives no hardware, no percentile beyond the median, and says it "collaborated tightly with partners across the platform" to get there [2].
- Windows App SDK 1.6 measured a "50% reduction in startup time" with NativeAOT for its Contoso Camera sample. It gave no milliseconds [6].
- A third-party test on a 2-core, 8 GB Windows 11 VM measured process start to window display: WinUI 3 NativeAOT self-contained 0.09 s and 0.18 s; WPF on .NET 8 0.50–0.68 s; WPF ReadyToRun 0.49–0.57 s [7]. The sample size is two or three runs, so treat this as weak evidence.
- Microsoft's Rust WinUI 3 gallery (windows-reactor) reports 160 ms to first window, against 465 ms for the C# JIT version. Hardware is not stated [34].
- WPF cannot be NativeAOT-compiled (dotnet/wpf#3811 is open) [54]. ReadyToRun still runs the JIT [55]. One report measured an empty .NET 6 R2R app at "2-3 seconds" for the first launch after boot and "around 250-300 ms" after that [56]. WPF breaks the PRD "no JIT" rule.
- Flutter's default Impeller renderer took 1,232 ms to first frame for a minimal Windows app, against 165 ms with Skia [66]. Flutter scores 0.
- Avalonia publishes no Windows NativeAOT startup numbers [49]. Its Skia, HarfBuzz and ANGLE DLLs are about 18.8 MB on x64 and must load before the first frame [81]. I estimate this makes cold start slower than WinUI (estimate).
- Measured here (warm, dev PC): a Rust windows-rs Win32 window with Direct3D 11, a DXGI flip swap chain, Direct2D and DirectWrite reached its first presented frame in a median of 170.6 ms. The p95 was 223.4 ms (n = 10, so p95 is the maximum) [1]. This is a floor for any GPU-drawn stack on this PC with this method. It includes process creation, Direct3D device creation on a hybrid Intel and NVIDIA GPU, a vsync-paced Present, and a DwmFlush (estimate of the breakdown).

### 2. Binary size, installer size, and memory

- Rust Win32 + Direct2D hello: one 128,000-byte exe that needs only system DLLs. Working set 55.0 MB and private bytes 59.8 MB, median after 2 s idle [1]. The harness reported a 104 MB folder, but that folder holds Cargo build files, not shipped files [1].
- A 55 MB working set for an empty Direct3D window suggests that the GPU driver stack takes a large fixed share of the PRD's 120 MB budget in every GPU-drawn stack (estimate).
- windows-reactor gallery: 3.34 MB deploy, 109.5 MB working set, 101.0 MB private memory [34].
- The Microsoft.WindowsAppSDK.WinUI 2.3.9 NuGet package is 58.75 MB for all architectures together [23]. A self-contained app copies the framework files next to the exe, and `dotnet publish` cannot make a single-file exe for WinUI 3 [22]. A Store build that depends on the shared framework package avoids this cost (estimate).
- PDFium is 3.9 MB compressed for win-x64 and 3.6 MB for win-arm64 [31]. This cost is the same for every stack.
- SumatraPDF, a C++ Win32 PDF reader, ships a 10.56 MB x64 installer [76].

### 3. Native Windows 11 look

- WinUI 3 is Microsoft's Fluent control set. It runs on Windows 10 1809 and later, including Windows 11 [20]. It supports Mica and Mica Alt, with a solid fallback color on builds below 22000 (Windows 10) [8]. It supports custom title bars with interactive content [19]. Windows App SDK 2.0 added `SystemBackdropElement` [4].
- Any Win32 window can request Mica through `DWMWA_SYSTEMBACKDROP_TYPE` from Windows 11 build 22621 [9]. Mica is therefore not a differentiator. Fluent controls are.
- Qt: the Windows 11 Widgets style arrived in 6.7. The Qt Quick FluentWinUI3 style "is under development, and some controls are not yet supported" [64].
- Avalonia's Fluent theme is "inspired by" Fluent, and Mica works with a Windows 10 fallback [53].
- WPF's Fluent theme is "still in experimental mode" [80].
- Slint's default style on Windows is `fluent`, but it has no Mica support [40]. egui, iced, gpui and Xilem draw their own non-Windows look (estimate based on their docs).

### 4. Accessibility

- WinUI 3: "most built-in controls already provide UI Automation support". Custom controls (for the page view) derive an `AutomationPeer` and can implement any provider pattern [14].
- Raw Win32 custom controls: "you need to implement UI Automation providers for those controls" [15]. AccessKit's Windows adapter, the reference Rust implementation, is about 3,760 lines and implements 14 UIA interfaces. Its raw-Win32 example is 317 lines [38]. AccessKit "doesn't yet support rich text or hypertext" [38].
- Rust toolkits:
  - egui: AccessKit always on [45].
  - Slint 1.18: AccessKit 0.24. Text inputs now expose content and selection [40].
  - iced: no accessibility; PR #3281 closed unmerged [42].
  - Zed (gpui) on Windows: "absolutely inaccessible for screen reader users", issue open [43].
- Avalonia: UIA exists but has open Narrator bugs. Text scaling is "not supported", with PR #20292 open, so the PRD's 200% text scale fails [50]. High contrast is detected but the Fluent theme has no high-contrast resources [53].
- Qt has used UI Automation since 5.11 [65].
- Flutter has open Narrator focus bugs (#121907) [67].
- Fyne's screen-reader support is off by default (build tag) [68]. Gio has no UIA code on Windows [70].
- SumatraPDF added UIA providers in 2026, but a tester reported "the main content was completely inaccessible" [77]. This shows the cost of retrofitting accessibility onto custom-drawn Win32.

### 5. Pen, touch, and ink

- WinUI 3: `PointerPointProperties` exposes pressure, tilt, twist, eraser and barrel button [12]. InkCanvas, InkPresenter and InkToolbar exist only in Experimental releases (2.3 to 2.5 Experimental). No Stable heading lists them [4]. Signatures and freehand need custom strokes drawn with Win2D, or must wait for InkCanvas to reach Stable.
- Win32 (C++ or Rust): `WM_POINTER` and `GetPointerPenInfo` give pressure (0–1024) and tilt [11]. `InkDesktopHost` and `IInkPresenterDesktop` put the system InkPresenter into a DirectComposition tree from Windows 10 [10]. Direct Manipulation gives off-thread pinch and pan [13]. This is the richest ink path, but it is the most code (estimate).
- WPF has InkCanvas [80].
- Avalonia reads pressure through Windows Ink and has pinch, but has no InkCanvas (#7004 open) [51].
- Slint has no pen-pressure API, and its pinch is trackpad-only on Windows [40]. winit 0.30 exposes pen pressure only as `Touch.force` (Rust agent report, see [41]).

### 6. Text input, IME, menus, drag and drop, share, print, and shell

- WinUI 3 has a TextBox with IME, and its drag-out uses `DataPackage.SetStorageItems` in `DragStarting` [18].
- Share and print are HWND-based COM interop in every stack: `IDataTransferManagerInterop::ShowShareUIForWindow` and `IPrintManagerInterop` [16]. C#, C++ and Rust (windows-rs) can call them (Rust: Rust agent report, see [32]).
- Explorer context menus need a native COM DLL that implements `IExplorerCommand`, registered in the MSIX manifest. Microsoft says "C++ is the usual choice" [17]. This DLL is native code in every stack, so it is the one place where Rust (or C++) is required anyway.
- Rust toolkits on winit 0.30 cannot start an OS drag, so a thumbnail cannot be dragged out to Explorer. winit 0.31 beta adds drag-out, for file paths only [41]. Slint's drag-out works only on its Qt backend [40].
- Avalonia has no print API (#12567 closed without one) [52].
- Fyne's IME has an open issue from 2020 [69].
- Qt uses the native print dialog and CF_HDROP drag-out [64][65].

### 7. Interop with PDFium, WIC, OCR, Windows AI, ONNX Runtime and qpdf

- C#: CsWinRT projections are AOT-compatible from CsWinRT 2.1.1 [60]. PDFium and qpdf have C APIs, which C# calls with `LibraryImport` (estimate: standard P/Invoke). The ONNX Runtime C# package works under NativeAOT since 1.25 after PR #27397, but it is not declared AOT-compatible [61].
- C++: direct calls to everything.
- Rust:
  - The `windows` crate covers WIC, Direct2D, Windows.Media.Ocr and UIA interfaces [32].
  - pdfium-render 0.9.4 is MIT or Apache-2.0 [31].
  - ort is still 2.0.0-rc.13 [46].
  - qpdf-rs does not test `aarch64-pc-windows-msvc` [47].
  - Windows AI APIs need bindings generated from the Windows App SDK metadata (unverified).
- Windows AI APIs require an app manifest with the `systemAIModels` capability, so the app must be packaged. Most of them need a Copilot+ NPU. Microsoft documents only C# and C++ [26].
- Go has no general WinRT projection: winrt-go covers Bluetooth only [71]. OCR and Windows AI would need hand-written bindings.

### 8. ARM64, packaging, maturity, and license

- Windows App SDK:
  - 2.0 shipped 2026-04-29. The current release is 2.5.1 (2026-09-16), and 1.8 servicing ended 2026-09-24 [3].
  - The WinUI repo is MIT and can now be built from source [21].
  - The NuGet package requires license acceptance; its license file was not read (unverified).
- NativeAOT for WinUI 3 has been supported since Windows App SDK 1.6 [5].
- C++/WinRT 3.0 shipped on 2026-05-22 with C++20 modules. On a Windows Terminal prototype, builds took 22 minutes before and 19 after, and precompiled headers used "1-2GB" per project [24]. This is the C++ build-cost evidence.
- Rust:
  - `aarch64-pc-windows-msvc` is Tier 1 from Rust 1.91 [37].
  - windows-rs moved core crates to 0.100 with breaking module changes. The `windows` umbrella crate is still 0.62.2 [32].
  - windows-reactor is "currently a preview" [33]. The old Rust WinAppSDK crate was archived because the SDK was "too heavily tied to .NET and Visual Studio" [36].
- Slint is GPL-3.0, royalty-free with mandatory attribution, or commercial [39].
- Qt 6.12 is LTS but "will be the last version to support Windows 10" [62]. Some modules are GPL-only, including the Qml Compiler [63].
- Xilem's README says "experimental" [44].

### 9. Development effort (2 engineers, about 15 weeks)

- The PRD UI needs about 20 control types: tabs in the title bar, a command bar, menus and context menus, a text box with IME, a number box, a slider, a combo box, a check box, a virtualized thumbnail list with drag reorder, a tree, dialogs, tooltips, progress, an info bar, and a zoomable scroll view (estimate from PRD sections "UX" and "Scope").
- WinUI 3 provides all of these with accessibility [14][19][20]. Shipped apps include PowerToys and parts of the Windows shell [20], the Run dialog [2], and Files (45,000+ GitHub stars, C#) [78].
- Custom Win32, in C++ or Rust, means building each control plus its UIA provider, keyboard handling, high contrast and text scaling. The PRD estimates 2 to 3 times the UI effort. The AccessKit size above [38] and SumatraPDF's accessibility state [77] support that estimate. I estimate this consumes most of the 15 weeks before any PDF feature work (estimate).
- Hybrid Rust core + C# shell:
  - The core would mostly wrap C and C++ libraries (PDFium, qpdf, libheif), and C# is already memory-safe. Rust adds no safety the shell lacks (estimate).
  - Costs: two toolchains, a C ABI layer (csbindgen 1.9.8 exists [48]), and two ARM64 builds.
  - Benefit: none on launch, because the core DLL loads after the first frame (estimate).

## Pending measurements

Shared method (one harness for every stack): `docs/research/proofs/ui-stack/measure.py`.

- The harness reads QueryPerformanceCounter just before `CreateProcess`.
- The app writes QPC after its first frame is presented and `DwmFlush()` returns. WinUI uses `CompositionTarget.Rendered` [30]. Win32 uses the first `Present`.
- QPC values are comparable across processes [29].
- The harness reads working set and private bytes after 2 s idle, then kills the app.
- `--selftest` passes on the dev PC.
- `--cold` purges the standby list. It needs an elevated prompt and was not used.

Results so far (dev PC: i7-11800H, 32 GB, Windows 10 22H2, not the PRD laptop):

| Proof | Status | Mode | n | Median | p95 | Working set | Private | Shipped size |
|---|---|---|---|---|---|---|---|---|
| `rust-win32-d2d` (windows 0.62.2, Direct3D 11 + Direct2D) | Measured [1] | Warm (after one discarded run) | 10 | 170.6 ms | 223.4 ms | 55.0 MB | 59.8 MB | 0.12 MB exe |
| `winui3-csharp-aot` (.NET 10, Windows App SDK 2.5.1, unpackaged, self-contained) | Pending: build failed | — | — | — | — | — | — | — |

The WinUI build failed at the NativeAOT link step. The error was `'vswhere.exe' is not recognized`. vswhere.exe exists in `C:\Program Files (x86)\Microsoft Visual Studio\Installer` but is not on PATH.

To run:

```sh
# Rust (built and measured)
cd docs/research/proofs/ui-stack/rust-win32-d2d && cargo build --release
python ../measure.py --exe target/release/pfw-bench-rust-win32-d2d.exe --runs 20 --csv results.csv

# C# WinUI 3 NativeAOT (pending)
export PATH="/c/Program Files (x86)/Microsoft Visual Studio/Installer:/c/Users/Armaan/AppData/Local/Microsoft/dotnet:$PATH"
cd docs/research/proofs/ui-stack/winui3-csharp-aot && dotnet publish -c Release -r win-x64 -p:Platform=x64
python ../measure.py --exe bin/Release/net10.0-windows10.0.26100.0/win-x64/publish/PfwBench.exe --runs 20 --csv results.csv

# Cold runs (elevated prompt, reference and low-end laptops)
python measure.py --exe <exe> --runs 20 --cold
```

Still to measure, in order:

1. WinUI 3 hello warm, on the dev PC, with the same harness.
2. Both hellos cold (`--cold` or one reboot per run) on the i5 12th-gen laptop and on the 4 GB eMMC laptop, with 20 or more runs.
3. Both hellos plus PDFium rendering of the first page of a 50 MB PDF (week-2 spike).
4. A Slint hello, which was dropped when the owner cut scope.

Decision rule for measurement 3, using cold p95 on the low-end laptop:

- WinUI at or below 340 ms (400 ms minus 15% margin): keep WinUI.
- WinUI above 400 ms, and WinUI minus Win32 more than 150 ms: the framework is the problem. C++/WinRT will not fix it, and only a Win32 + Direct2D launch window will (estimate).
- Both above 400 ms: disk or PDFium is the problem, and no stack choice fixes it.

## Confidence

- **WinUI 3 wins on features, accessibility and effort: high.** Primary Microsoft docs and release notes support every cell [4][14][16][20].
- **WinUI 3 meets 400 ms p95 cold on the low-end PC: low.** The only data are a median on unstated hardware [2], a 2-core VM test with two or three runs [7], and no cold measurement of our own.
- **Raw Win32 + Direct2D is the launch floor: medium.** It is measured warm on one PC only [1], and WinUI has not yet been measured with the same method.
- **Rust toolkits fail hard requirements: high for iced, gpui and Slint on drag-out and pen; medium for egui.** egui accessibility is on, but its look and drag-out still fail [40][41][42][43][45].
- **windows-reactor is not ready for a 15-week ship: medium.** It is preview by its own docs [33] with a Windows 10 crash [35]. Gaps in drag-source and ink wrappers come from a source search by a research agent (unverified).
- **Effort multipliers: low.** These are estimates, with no controlled study.

## Conflicts with the PRD

1. **Target OS.** Windows 10 reached end of support on 2025-10-14 [27]. Consumer ESU runs to 2027-10-12 [28]. On Windows 10, Mica falls back to a solid color [8] and the DWM backdrop attribute does not exist [9]. Qt 6.12 is Qt's last Windows 10 release [62]. windows-reactor crashes on Windows 10 when framework-dependent [35]. Windows App SDK still supports 1809 [3].
2. **Launch reference.** The PRD's 94 ms reference is a median, not a p95. Its hardware is unknown, and Microsoft needed platform-team help to reach it [2]. It does not prove p95 under 400 ms on a 4 GB eMMC PC.
3. **C++/WinRT fallback.** The PRD fallback moves the launch window to C++/WinRT. That still loads WinUI. If WinUI itself is the cost, the fallback does not help (estimate). Measure WinUI against the Win32 floor before relying on it.
4. **Ink.** InkCanvas is Experimental-only [4]. Plan signatures and freehand on Win2D pointer input [12], not on InkCanvas.
5. **Memory.** An empty Direct3D window uses 55 MB working set here [1], and the Rust WinUI gallery uses 109.5 MB [34]. The 120 MB budget for one 20-page PDF is at risk (estimate).
6. **Distribution.** Windows AI APIs need a packaged app with the `systemAIModels` capability [26]. A non-Store direct-download installer must still be MSIX or a sparse package to use them.
7. **License.** The PRD lists WinUI as MIT. The source repo is MIT [21], but the NuGet package has its own license file (unverified).

## What we should do

1. Keep C# + WinUI 3 + NativeAOT on Windows App SDK 2.5.x and .NET 10 as the primary stack.
2. Put vswhere on PATH and measure the WinUI hello with `measure.py` on the dev PC: 20 warm runs. This takes about 15 minutes.
3. In the week-2 spike, run both hellos plus the PDFium first page cold on both PRD laptops, then apply the decision rule above.
4. Hold C++/WinRT + WinUI 3 as the fallback, and use it only if the measured gap is .NET-specific.
5. Re-check windows-reactor at the week-2 gate. If it leaves preview and wraps drag source and pen input, it becomes the Rust path within the same WinUI design.
6. Use Rust (or C++) only where native code is mandatory: the `IExplorerCommand` context-menu DLL [17]. Do not build a hybrid Rust core.
7. Build signatures and freehand on Win2D with pointer pressure. Adopt InkCanvas only after it reaches a Stable release.
8. Ask the owner to decide on Windows 10 support (PRD open question) now. It changes test scope and Mica behavior.

## Open questions

- What is the cold p95 of a WinUI 3 window on the 4 GB eMMC laptop, unpackaged self-contained versus packaged framework-dependent? The WinUI performance agent owns the packaging comparison.
- Is the Windows App Runtime framework package usually already in memory on Windows 11 PCs, which would make our "cold" launch warm? (unverified)
- What do the redistribution terms in the Windows App SDK NuGet license file say?
- How much does Direct3D device creation cost on a low-end Intel iGPU, compared with the 171 ms floor measured here?
- Will windows-reactor ship drag source, pen and ink wrappers, and Windows 10 framework-dependent support, before week 8?

## Sources

1. This study, Rust hello-window benchmark, 2026-10-06: `docs/research/proofs/ui-stack/measure.py` and `rust-win32-d2d/`. Raw CSV in the session scratchpad.
2. The new Run dialog: faster, cleaner, more capable (Microsoft, 2026-05-01), https://devblogs.microsoft.com/commandline/the-new-run-dialog-faster-cleaner-and-more-capable
3. Windows App SDK release channels and lifecycle, https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/release-channels
4. Windows App SDK 2.0 release notes, https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/release-notes/windows-app-sdk-2-0
5. Windows App SDK 1.6 release notes (Native AOT), https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/release-notes/windows-app-sdk-1-6
6. What's new in Windows App SDK 1.6, https://blogs.windows.com/windowsdeveloper/2024/09/04/whats-new-in-windows-app-sdk-1-6/
7. Startup comparison of WPF, WinUI 3 NativeAOT and native C++ (zenn.dev, 2025-05-23), https://zenn.dev/suusanex/articles/f642d91df412c9
8. Mica material, https://learn.microsoft.com/en-us/windows/apps/design/style/mica
9. DWMWINDOWATTRIBUTE, https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute
10. IInkPresenterDesktop, https://learn.microsoft.com/en-us/windows/win32/api/inkpresenterdesktop/nn-inkpresenterdesktop-iinkpresenterdesktop
11. POINTER_PEN_INFO, https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-pointer_pen_info
12. PointerPointProperties (Windows App SDK), https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.input.pointerpointproperties
13. Direct Manipulation, https://learn.microsoft.com/en-us/windows/win32/directmanipulation/direct-manipulation-portal
14. Custom automation peers (WinUI), https://learn.microsoft.com/en-us/windows/apps/design/accessibility/custom-automation-peers
15. UI Automation providers overview, https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-providersoverview
16. Display WinRT UI objects that depend on CoreWindow, https://learn.microsoft.com/en-us/windows/apps/develop/ui/display-ui-objects
17. Add a File Explorer context menu command to a packaged desktop app, https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/integrate-packaged-app-with-file-explorer
18. Drag and drop (Windows apps), https://learn.microsoft.com/en-us/windows/apps/develop/data/drag-and-drop
19. Title bar customization, https://learn.microsoft.com/en-us/windows/apps/develop/title-bar
20. WinUI 3 overview, https://learn.microsoft.com/en-us/windows/apps/winui/winui3/
21. microsoft-ui-xaml repository, https://github.com/microsoft/microsoft-ui-xaml
22. Windows App SDK deployment guide for self-contained apps, https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/self-contained-deploy/deploy-self-contained-apps
23. Microsoft.WindowsAppSDK.WinUI on NuGet, https://www.nuget.org/packages/Microsoft.WindowsAppSDK.WinUI
24. C++/WinRT 3.0.260520.1 release notes, https://github.com/microsoft/cppwinrt/releases/tag/3.0.260520.1
25. DesktopWindowXamlSource class, https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.hosting.desktopwindowxamlsource
26. Get started with Windows AI APIs, https://learn.microsoft.com/en-us/windows/ai/apis/get-started
27. Windows 10 Home and Pro lifecycle, https://learn.microsoft.com/en-us/lifecycle/products/windows-10-home-and-pro
28. Windows 10 Extended Security Updates, https://www.microsoft.com/en-us/windows/extended-security-updates
29. Acquiring high-resolution time stamps, https://learn.microsoft.com/en-us/windows/win32/sysinfo/acquiring-high-resolution-time-stamps
30. CompositionTarget class (Windows App SDK), https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.media.compositiontarget
31. pdfium-binaries releases (chromium/8086) and pdfium-render, https://github.com/bblanchon/pdfium-binaries/releases, https://crates.io/crates/pdfium-render
32. windows-rs release 74 and 0.100 notes, https://github.com/microsoft/windows-rs/issues/4867
33. windows-reactor documentation, https://github.com/microsoft/windows-rs/blob/master/docs/crates/windows-reactor.md
34. windows-rs issue 4479 (Reactor gallery size, launch and memory), https://github.com/microsoft/windows-rs/issues/4479
35. windows-rs issue 4924 (Reactor on Windows 10), https://github.com/microsoft/windows-rs/issues/4924
36. windows-app-rs (archived), https://github.com/microsoft/windows-app-rs
37. Announcing Rust 1.91.0, https://blog.rust-lang.org/2025/10/30/Rust-1.91.0/
38. AccessKit repository and Windows adapter, https://github.com/AccessKit/accesskit
39. Slint Royalty-free License 2.0, https://github.com/slint-ui/slint/blob/master/LICENSES/LicenseRef-Slint-Royalty-free-2.0.md
40. Slint changelog, https://github.com/slint-ui/slint/blob/master/CHANGELOG.md
41. winit PR 4571 (drag and drop source), https://github.com/rust-windowing/winit/pull/4571
42. iced PR 3281 (accessibility, closed), https://github.com/iced-rs/iced/pull/3281
43. Zed issue 41138 (Windows screen readers), https://github.com/zed-industries/zed/issues/41138
44. Xilem repository, https://github.com/linebender/xilem
45. egui PR 7701 (AccessKit always on), https://github.com/emilk/egui/pull/7701
46. ort crate, https://crates.io/crates/ort
47. qpdf-rs, https://github.com/ancwrd1/qpdf-rs
48. csbindgen, https://github.com/Cysharp/csbindgen
49. Avalonia Native AOT, https://docs.avaloniaui.net/docs/deployment/native-aot
50. Avalonia PR 20292 (text scaling), https://github.com/AvaloniaUI/Avalonia/pull/20292
51. Avalonia issue 7004 (inking), https://github.com/AvaloniaUI/Avalonia/issues/7004
52. Avalonia issue 12567 (printing), https://github.com/AvaloniaUI/Avalonia/issues/12567
53. Avalonia Windows platform guide, https://docs.avaloniaui.net/docs/platform-specific-guides/windows
54. dotnet/wpf issue 3811 (trimming), https://github.com/dotnet/wpf/issues/3811
55. ReadyToRun compilation, https://learn.microsoft.com/dotnet/core/deploying/ready-to-run
56. dotnet/runtime issue 78379 (WPF startup), https://github.com/dotnet/runtime/issues/78379
57. dotnet/maui issue 31227 (Windows NativeAOT), https://github.com/dotnet/maui/issues/31227
58. How Uno Platform works, https://platform.uno/docs/articles/how-uno-works.html
59. dotnet/winforms issue 4649 (trimming and AOT), https://github.com/dotnet/winforms/issues/4649
60. CsWinRT AOT and trimming, https://github.com/microsoft/CsWinRT/blob/master/docs/aot-trimming.md
61. onnxruntime PR 27397, https://github.com/microsoft/onnxruntime/pull/27397
62. Qt 6 supported platforms, https://doc.qt.io/qt-6/supported-platforms.html
63. Qt licensing, https://doc.qt.io/qt-6/licensing.html
64. Qt FluentWinUI3 style and QPrintDialog, https://doc.qt.io/qt-6/qtquickcontrols-fluentwinui3.html, https://doc.qt.io/qt-6/qprintdialog.html
65. New features in Qt 5.11 (UI Automation), https://wiki.qt.io/New_Features_in_Qt_5.11
66. flutter issue 191860 (Windows startup), https://github.com/flutter/flutter/issues/191860
67. flutter issue 121907 (Narrator), https://github.com/flutter/flutter/issues/121907
68. Fyne PR 6105 (accessibility), https://github.com/fyne-io/fyne/pull/6105
69. Fyne issue 618 (IME), https://github.com/fyne-io/fyne/issues/618
70. Gio Windows backend source, https://github.com/gioui/gio/blob/main/app/os_windows.go
71. winrt-go, https://github.com/saltosystems/winrt-go
72. lxn/walk, https://github.com/lxn/walk
73. Wails installation (WebView2), https://github.com/wailsapp/wails/blob/master/website/docs/gettingstarted/installation.mdx
74. Tauri repository, https://github.com/tauri-apps/tauri
75. Electron repository, https://github.com/electron/electron
76. SumatraPDF download page, https://www.sumatrapdfreader.org/download-free-pdf-viewer
77. SumatraPDF issue 321 (screen readers), https://github.com/sumatrapdfreader/sumatrapdf/issues/321
78. Files app repository, https://github.com/files-community/Files
79. React Native for Windows, https://github.com/microsoft/react-native-windows
80. WPF Fluent theme doc and InkCanvas, https://github.com/dotnet/wpf/blob/main/Documentation/docs/using-fluent.md, https://learn.microsoft.com/dotnet/api/system.windows.controls.inkcanvas
81. Avalonia.Skia and SkiaSharp native assets on NuGet, https://www.nuget.org/packages/Avalonia.Skia
