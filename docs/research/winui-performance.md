# WinUI 3 launch performance

Date: 2026-10-06 · Author: Research agent: WinUI 3 performance · Status: draft

Scope: can a C# WinUI 3 app show the first PDF page in under 400 ms p95 cold and under 150 ms p95 warm? No WinUI build ran in this session. The owner's budget ran out before the deep launch breakdown, so every launch number below comes from a published source, or it is an estimate.

## Summary

- **Current stack (verified):** Windows App SDK 2.5.1, .NET 10 LTS (10.0.12), CsWinRT 2.3.1 and Win2D 1.4.0. NativeAOT works for WinUI 3 from Windows App SDK 1.6, on x64 and Arm64 [1][3][4][6][8][9].
- **The PRD's source does not support its targets.** The Run dialog's 94 ms is a median time-to-show from telemetry. It came from Windows 11 Insider builds, a very small dialog and OS-side tuning. The post gives no p95, no cold or warm split and no hardware [34]. No Microsoft source gives cold or warm p95 for any WinUI 3 app.
- **Known costs:**
  - An unpackaged, framework-dependent build adds 100–300 ms. This applies on Windows 10 and on Windows 11 before 24H2 [24][25].
  - JIT doubles start time compared with AOT [22].
  - Each XAML element costs about 1 ms [21].
  - Windows 10 itself is slow. One third-party test measured WinUI at 813 ms on Windows 10 against 308 ms on Windows 11 [31].
- **Design that can plausibly meet the targets (estimate):**
  - Ship as packaged MSIX or self-contained, compiled with NativeAOT.
  - Build the window in code with a Grid and an Image.
  - Start the PDFium render on a worker thread in `Main`.
  - Defer Win2D and all chrome until after the first frame.
  - Use single-instance redirection and keep the process alive after the last window closes. This keeps warm launches under 150 ms.
- **The C++/WinRT fallback is weak.** It keeps the same XAML framework, so it can only recover the managed share of startup (estimate: 10–40 ms). If WinUI misses the gate, a better fallback is a Win32 window that draws the first page with Direct2D or DirectComposition, with XAML loaded after the first frame (estimate).

## Findings

### 1. Current versions and minimum Windows

| Component | Stable today | Date | Notes | Source |
| --- | --- | --- | --- | --- |
| Windows App SDK | 2.5.1 | 2026-09-16 | 2.x serviced to 2027-04-29. 1.8 left servicing on 2026-09-24. Experimental: 2.5.4. | [1] |
| .NET | 10 LTS, patch 10.0.12 | 2026-09-08 | Supported to 2028-11-14. .NET 8 and 9 end on 2026-11-10. | [3] |
| CsWinRT | 2.3.1 | 2026-07-23 | Adds a .NET 9+ WinRT.Runtime that marshals non-blittable arrays under AOT. Drops .NET 6 and 7. A breaking 3.0 preview (2026-03-25) is a rewrite on .NET 10 and covers projections only. | [4][5] |
| Win2D | 1.4.0 | 2026-03-16 | Depends on `Microsoft.WindowsAppSDK.WinUI` ≥ 1.8.260204000 and CsWinRT 2.2.0. Ships win-x64, win-arm64 and win-x86 binaries. | [6][7] |

- NativeAOT for WinUI 3 starts at Windows App SDK 1.6, with `PublishAot` [8].
- .NET NativeAOT targets Windows x64 and Arm64 on .NET 8, and x86 as well on .NET 9 and later. It needs the Visual Studio "Desktop development with C++" workload [9].
- Windows App SDK runs on Windows 10 1809 and later. Microsoft supports it only on Windows releases that are still in support [1][10].
- Win2D 1.3.0 and later target 10.0.19041, which is Windows 10 2004 [7].
- .NET 10's supported-OS list names Windows 11 releases and Windows 10 21H2, 1809 and 1607 (Enterprise or IoT). It does not name Windows 10 22H2 Home or Pro [11].
- Windows 10 Home and Pro 22H2 left support on 2025-10-14 [12]. Consumer ESU runs to 2027-10-12 [13].
- Mica needs build 22000 or later. On older builds it shows a solid color [14].
- Win2D 1.4.0 on Windows App SDK 2.x: the NuGet version range allows it, but no test confirms it works (unverified) [6].

### 2. NativeAOT: requirements and known issues

**Requirements**

- Set `PublishAot=true` unconditionally (Windows App SDK 1.6 stable onward). Reference CsWinRT 2.1.1 or later [8].
- Classes that cross the WinRT ABI must be `partial`. Set `CsWinRTAotWarningLevel` to 2 [15].
- `{Binding}` is not AOT-safe. Use `x:Bind`. For non-x:Bind binding, mark source classes `partial` and add `[GeneratedBindableCustomProperty]`. Root reflection-only types with `TrimmerRootDescriptor` [8][15].
- A shipped Microsoft example is PowerToys Command Palette, the code base behind the new Run dialog. It sets `IsAotCompatible`, `CsWinRTAotOptimizerEnabled`, `CsWinRTAotWarningLevel=2`, `WindowsAppSDKSelfContained=true`, `DISABLE_XAML_GENERATED_MAIN` and `EventSourceSupport=true` [16].

**Open AOT bugs**

- Templated controls are hard to make AOT-safe because `x:Bind` does not work in resource templates (microsoft-ui-xaml #10214).
- Binding in `ItemsPanelTemplate` (#10070).
- Groups in `CollectionViewSource` (#10881).
- `SemanticZoom` (#11149).
- Custom controls (#11065).
- `NonClientRegionsChangedEventArgs` (#10835).
- `AnnotatedScrollBarLabel` (#10188).

All seven are in [17].

**Fixed or limited bugs**

- A GC hang was fixed in .NET 9 RC2 [15].
- A .NET 10 AOT hang (microsoft-ui-xaml #10882, CsWinRT #2121) affects debug AOT publishes only [18].
- `.appxsym` files from AOT publishes lack the exe's PDB (WindowsAppSDK #6208, open). This puts crash symbolication at risk [19].

**Win2D**

- Win2D projections have been AOT-safe since the August 2024 previews (Win2D #960) and in stable 1.3.0 and later [20][7].
- `CanvasTextLayout.LineMetrics` failed under AOT until CsWinRT 2.3.0. Use CsWinRT 2.3.1 [20][4].

### 3. Startup cost breakdown

Microsoft describes WinUI startup in these stages [21]:

1. Process launch.
2. App constructor and `App.xaml` parse.
3. `OnLaunched`: create the window, set its content, call `Activate`.
4. Page `InitializeComponent`.
5. Layout. `ApplyTemplate` is "typically the bulk of layout time".
6. Render, then first frame.

| Stage | Published evidence | Source |
| --- | --- | --- |
| Process start and loader | No WinUI-specific number (unverified). | n/a |
| .NET runtime | AOT cut start time by 50% in Microsoft's Contoso Camera sample, with no absolute time given. UWP on .NET 9 AOT starts within about 5% of .NET Native. | [22][23] |
| Windows App SDK bootstrap | Unpackaged framework-dependent builds were 100–300 ms slower than self-contained (1.6, Windows 11 22H2). Windows App SDK hands Dynamic Dependencies to the OS only on Windows 11 24H2 and later. Older systems, including Windows 10, use a Detours-based polyfill. The reporter said in October 2025 that the cost had "reduced drastically", with no number. Packaged apps do not use the bootstrapper. Unpackaged self-contained apps use UndockedRegFreeWinRT. | [24][25][26][27] |
| WinRT calls on the launch path | A user reported 15–25 ms per call for `ApplicationData.Current.*Folder` and `Package.Current.InstalledLocation` on an i9. Microsoft disputes the size and added `*Path` properties (PR #6731). | [28] |
| XAML parse and layout | Rule of thumb: 1 ms per element. Windows App SDK 2.3.1 added startup optimizations, some of them opt-in through `XamlChangeId`. In the WinUI portion of File Explorer launch: 25% less time in WinUI code, 45% fewer function calls, 41% fewer allocations. | [21][2][29] |
| DirectX device | WinUI creates its D3D and D2D device on a background thread (the "Create graphics device" event). 2.3.1 cut D3D lock waits at startup. No published duration. Win2D creates a second device. | [30][2] |
| Mica | Samples the wallpaper once. No published cost. | [14] |
| First frame present | No published number. | n/a |
| End to end, third party | WinAppLaunchCompare measured time to `TextBlock.Loaded` with WinUI 1.7, .NET 9 JIT, unpackaged and framework-dependent. Mean of 10 runs, with the fastest and slowest dropped: Windows 10 813 ms, Windows 11 x64 308 ms, Windows 11 Arm64 498 ms. WinForms on the same machines: 248, 231 and 137 ms. | [31] |

A secondary site claims "30% average cold-launch improvement", "40% less base memory" and "File Explorer 2.7 s to 1.4 s". None of these appear in the primary source (unverified) [32][29].

### 4. Published benchmarks

- **PRD citation checked:**
  - Windows Latest [33] cites a Microsoft devblog by Clint Rutkas, dated 2026-05-01 [34].
  - That post says the Run dialog is C#/WinUI 3 compiled with .NET AOT. Its median time-to-show is 94 ms, against 103 ms for the old dialog.
  - Microsoft "added a measure briefly to the dialog" to get these numbers, so they come from telemetry.
  - The post gives no p95, no cold or warm split and no hardware. The feature is opt-in on the Insider Experimental channel.
  - Microsoft says it "collaborated tightly with partners across the platform", so OS-side changes contributed.
  - The post does not say whether the Run process stays resident (unverified).
- **Other Microsoft numbers:**
  - Contoso Camera: AOT gave 50% faster start, an app package about 8 times smaller when framework-dependent and about 2 times smaller when self-contained [22].
  - UWP on .NET 9 AOT is within about 5% of .NET Native [23].
  - File Explorer: the WinUI-portion figures in section 3 [29].
  - Photos added a setting that runs it "in the background ... to improve the app startup speed" [35].
- **Not found:** Microsoft absolute cold or warm launch times for WinUI 3, with or without AOT (unverified that none exist).
- **Third-party measurements:** WinAppLaunchCompare (section 3) [31]. No third-party WinUI NativeAOT launch numbers were found.

### 5. Window-first launch patterns

- **Show the window first.** Activate the window with light content in `OnLaunched` and move other work off the startup path [21].
- **Defer elements.** `x:Load` and `x:DeferLoadStrategy` delay creation. A collapsed element is still created [21].
- **Keep resources light.** Merging `XamlControlsResources` loads the Fluent styles. Its cost is unpublished (pending measurement P3).
- **Opt in to WinUI performance changes.** Call `Microsoft.UI.Xaml.Settings.XamlOptionalChanges.EnableChange` before `Application.Start` [36]. The WinUI source defines `DefaultStyleOptimizations`, `OptimizeApplyStyles`, `IconNoGridOptimization` and `DeferContextFlyoutInit` [37]. Those names are not checked against the 2.5.1 build (unverified).
- **No splash screen.** It does not get the page on screen sooner (estimate).
- **Single-instance redirection:**
  - Call `FindOrRegisterForKey` and then `RedirectActivationToAsync` in a custom `Main`. Wait with `CoWaitForMultipleObjects` so the STA does not block [38][39].
  - The redirect cost is not published (unverified).
  - Redirection does not reliably pass foreground rights. Call `SetForegroundWindow` [40].
- **Multi-select in Explorer.** For packaged Win32 apps, Explorer starts N processes when the user opens N files. Each process gets all N files (WindowsAppSDK #5066, open; fix PR #6278, open) [41]. This affects the PRD's "Open with" on multi-select.
- **Keep a process warm.**
  - Photos runs in the background at startup [35]. Edge Startup boost does the same [42].
  - Proposal: after the last window closes, keep the process alive for N minutes (estimate). This turns most repeat launches into hot launches.
- **File activation.**
  - Packaged apps declare file types in the package manifest. `AppInstance.GetActivatedEventArgs()` returns `Kind == File` [43].
  - Unpackaged apps get the file path on the command line.

### 6. Page view: Win2D and Direct2D

**`CanvasVirtualControl`**

- Built on `VirtualSurfaceImageSource`. It redraws only invalidated regions and works inside a `ScrollViewer` [44].
- Two WinUI 3 bugs are open:
  - The visible region is clamped to the window's first size (Win2D #983).
  - The control's size becomes 0 after maximize (Win2D #986).
- One affected user moved to `CompositionVirtualDrawingSurface` [45][53].

**Other options**

- `CanvasControl` redraws the whole surface. It is a poor fit for a 500-page document (estimate).
- `SwapChainPanel` and `CanvasSwapChainPanel` give full control, but the app then owns scrolling, input and DPI. This is the most work (estimate) [46].

**Proposal (estimate)**

- PDFium renders tiles on worker threads into CPU bitmaps.
- XAML or Composition shows the tiles, as `Image` tiles or a `CompositionVirtualDrawingSurface`.
- Composition scrolls the tiles. Nothing is redrawn per frame.
- The first page appears as a `WriteableBitmap` in an `Image`. The app needs no D3D device of its own for the first pixel.

**Device cost**

- No published duration. WinUI already creates its own device on a background thread [30].
- Creating the device on the discrete GPU of a hybrid laptop could add delay (unverified).

### 7. Cold-launch measurement method

**Definitions.** A launch is cold when the app's pages are not in memory, for example after a reboot. It is warm when the pages are in the standby list [47].

**Getting a true cold launch**

1. Reboot.
2. Wait for the logon to go idle.
3. Launch the app.

**Approximate cold launch.** Purge the standby list. This needs admin rights. Use RAMMap's "Empty Standby List" [52], or `NtSetSystemInformation`, which is undocumented (unverified).

**Prefetch.** Leave SysMain at its default. Users have it on. Measure the first launch after install separately.

**Timestamps**

- QPC values are consistent across processes [48].
- The harness takes T0 just before `ShellExecuteEx`.
- The app reports its QPC at the first `CompositionTarget.Rendered` after the page is set [49].
- Add up to 1 vsync for DWM (estimate).
- Validate with WPR. On this PC, `wpr -profiles` lists `XAMLActivity`, `DesktopComposition`, `GPU` and `DiskIO` (measured: WPR 10.0.19041).
- The WPA "XAML Frame Analysis" plug-in (Windows ADK 10.1.26100.1 and later) reports frame and "Create graphics device" durations [30].
- PresentMon reports present events [50].

**Number of runs.** These counts come from binomial order statistics (measured: Python computation):

- Below 59 runs, no distribution-free 95% upper bound on p95 exists.
- With 59 runs, the bound is the largest sample.
- With 100 runs, it is the 99th-smallest sample.
- With 200 runs, it is the 196th-smallest sample.

Use 59 runs per mode per build.

**CI setup**

- Use a dedicated, physical reference laptop as a self-hosted runner. VMs use WARP, which is not representative.
- Use auto-logon with an interactive session, AC power, a fixed power plan and paused updates.
- Expected time: about 10 minutes per build for warm and hot runs (estimate). Reboot-cold runs of 59 reboots take about 1.5 hours (estimate), so run them weekly.

### 8. The C++/WinRT fallback

**Evidence about the size of the gain**

- C# AOT WinUI reaches a 94 ms median in the Run dialog [34]. UWP on .NET 9 AOT is within about 5% of .NET Native [23].
- The framework, not the language, dominates. In the same harness on Windows 10, WinForms on .NET JIT took 248 ms and WinUI took 813 ms [31].
- C++/WinRT uses the same `Microsoft.UI.Xaml.dll`, the same device creation and the same DWM path. It removes only .NET runtime init and CsWinRT interop. The likely saving is 10–40 ms (estimate). No head-to-head measurement is published (unverified).

**Cost**

- The PRD estimates 2–3 times the UI build effort.
- A mixed C# and C++ process needs a WinRT component boundary through CsWinRT authoring, plus two build systems (estimate).
- C++/WinRT is still active. 3.0.260818.1 shipped in August 2026 [51].

**Bound the gain without building it.** Record a WPR trace of LaunchProbe. In WPA, compare CPU time in managed code (`LaunchProbe.exe` and WinRT interop) with CPU time in `Microsoft.UI.Xaml.dll` and the other framework DLLs. The managed share is the most that C++/WinRT can recover.

## Launch-path design

Assumptions:

- Packaged MSIX, NativeAOT, x64 or Arm64.
- The reference laptop: i5 12th gen, 8 GB, SSD, Windows 11.
- The user double-clicks a PDF in Explorer.
- Budgets are p95 values in ms.
- Every number in this table is an estimate. None is measured.

| # | Step | Cold p95 | Warm p95 (new process, files cached) | Basis |
| --- | --- | --- | --- | --- |
| 1 | Explorer `ShellExecute`, then package activation and process creation | 50 | 20 | Estimate |
| 2 | Loader maps the exe and Windows App SDK DLLs; AOT runtime starts and reaches `Main` | 70 | 20 | Estimate; AOT halves start time [22] |
| 3 | `GetActivatedEventArgs` and `FindOrRegisterForKey` | 10 | 5 | Estimate; pending P4 |
| 4 | Worker thread: PDFium opens the file and renders page 1 at screen size, in parallel with steps 5–6 | 150 | 60 | Estimate; PDF-engine research owns this |
| 5 | `Application.Start`, with an empty `App.xaml` and no `XamlControlsResources`. WinUI starts device creation on a background thread. | 80 | 35 | Estimate; [30] |
| 6 | `OnLaunched` builds a Grid, an Image and a placeholder in code, then calls `Activate` | 40 | 20 | Estimate; about 1 ms per element [21] |
| 7 | Bitmap goes into a `WriteableBitmap` and `Image.Source` | 10 | 5 | Estimate |
| 8 | Layout, render, first frame, DWM present | 40 | 25 | Estimate |
| | **Critical path:** max(1+2+3+5+6, 1+2+3+4) + 7 + 8 | **330** | **135** | Leaves 70 ms of reserve cold and 15 ms warm |

**Hot launch** (the PRD's "app already in memory"):

1. A second process starts and redirects to the running one (steps 1–3, about 50 ms).
2. The running process opens a window (about 20 ms) and renders the page (about 60 ms, starting at activation).
3. Steps 7 and 8 follow (about 30 ms).
4. Total: about 140 ms (estimate).

If the process keeps the first page of recent files cached, the total falls to about 100 ms (estimate).

**Deferred until after the first frame:**

- Win2D `CanvasDevice`
- `XamlControlsResources`, the toolbar, markup bar and sidebar (load with `x:Load` or code)
- Mica (pending P3)
- Thumbnails and text extraction
- Recent files and settings
- Telemetry and update checks
- OCR and AI models

**Risks**

- **Windows 10:** WinUI launched 2.6 times slower there in a third-party test [31].
- **Low-end PCs:** 4 GB, eMMC storage.
- **Defender:** it scans new binaries on their first launch.
- **Packaged activation:** its overhead is unknown.
- **Explorer multi-select:** it starts N processes [41].
- **Hybrid GPUs:** device creation on the discrete GPU could add delay.
- **Heavy first pages:** they blow step 4.
- **CsWinRT AOT bugs:** see section 2.
- **`XamlChangeId` names:** unverified on 2.5.1.

**What is measured and what is estimated**

- Measured: nothing in this table. Measured in this session: the WPR profile list, the p95 order-statistic table, and the harness self-test.
- Every budget above is an estimate until the pending measurements run.

## Pending measurements

No probe build or launch ran. The coordinator cancelled the launch breakdown for budget reasons. The toolchain is now installed: .NET SDK 10.0.401 and MSVC 14.50.

**Files**

- `docs/research/proofs/winui/LaunchProbe/`
  - `LaunchProbe.csproj`, `Program.cs`, `App.xaml`, `app.manifest`, `Package.appxmanifest` and `Assets/`
  - A code-built WinUI 3 window that shows a page-sized BGRA bitmap and reports QPC timestamps.
  - Variant switches in the file name: `xcr`, `toolbar`, `mica`, `opt`, `win2d` and `exit`.
  - Build switches: `-p:PublishAot=false`, `-p:ProbePackaged=false` and `-p:ProbeSelfContained=true`.
  - Not compiled yet.
- `docs/research/proofs/winui/harness/Measure-Launch.ps1`
  - Runs in Windows PowerShell 5.1, with Warm and Hot modes.
  - Writes CSV results and prints the median, p95 and the 95% upper bound on p95, with a pass/fail gate.
  - `-SelfTest` passed on the dev PC (measured: rank and percentile checks, plus a `WM_COPYDATA` loopback).

**Commands** (not run yet):

```powershell
# Build. Unpackaged and self-contained is the simplest; packaged needs signing or loose registration.
$env:DOTNET_ROOT = "$env:LOCALAPPDATA\Microsoft\dotnet"
& "$env:DOTNET_ROOT\dotnet.exe" publish docs\research\proofs\winui\LaunchProbe -c Release -r win-x64 -p:Platform=x64 -p:ProbePackaged=false -p:ProbeSelfContained=true
# Measure. Do not elevate.
.\docs\research\proofs\winui\harness\Measure-Launch.ps1 -Mode Warm -Runs 59 -Exe <publish dir>\LaunchProbe.exe
.\docs\research\proofs\winui\harness\Measure-Launch.ps1 -Mode Hot  -Runs 59 -Exe <publish dir>\LaunchProbe.exe
# Trace one run of each variant. Profiles are built into WPR.
wpr -start GeneralProfile -start XAMLActivity -start DesktopComposition -start DiskIO -filemode; <launch>; wpr -stop launch.etl
```

**Matrix.** Run 59 times per cell. Label each result "warm", "hot" or "first run after build". Cold runs need a reboot or admin rights and run later on the reference laptop.

| ID | What | Why |
| --- | --- | --- |
| P1 | NativeAOT vs JIT (`-p:PublishAot=false`) | Size the AOT gain on our own path |
| P2 | Packaged framework-dependent, packaged self-contained, unpackaged self-contained, unpackaged framework-dependent | Pick the deployment model; check the 100–300 ms bootstrap cost [24] |
| P3 | Flags: none, `xcr`, `xcr.toolbar`, `mica`, `opt`, `win2d` | Cost of styles, chrome, Mica, opt-in changes and the Win2D device |
| P4 | Hot mode (redirect into a resident process) | Check the 150 ms target |
| P5 | WPR and WPA XAML Frame Analysis for each P2 variant | Per-stage times and the "Create graphics device" duration |
| P6 | WPA CPU time split by module | Upper bound on the C++/WinRT gain |
| P7 | Windows 10 vs Windows 11; reference laptop and 4 GB eMMC PC; reboot-cold weekly | PRD gates |
| P8 | Replace the `.lprobe` bitmap with a real PDFium first-page render | End-to-end first page (with the PDF-engine agent) |
| P9 | Compile and run Win2D 1.4.0 on Windows App SDK 2.5.1, plus the open `CanvasVirtualControl` bugs #983 and #986 | Page-view choice |
| P10 | Explorer multi-select of 5 files: count processes and activations | Effect of #5066 |

## Confidence

| Finding | Confidence | Reason |
| --- | --- | --- |
| Versions and dates | High | Primary sources: Learn, GitHub releases, NuGet API |
| PRD source reads as a median from telemetry, not p95 cold | High | Read the primary devblog directly |
| Unpackaged framework-dependent costs 100–300 ms on pre-24H2 Windows | Medium | One user measurement plus the merged PR; later improvement not quantified |
| AOT halves start time | Medium | One Microsoft sample, no absolute numbers |
| Windows 10 is much slower for WinUI | Medium | One third-party harness, JIT, unpackaged |
| C++/WinRT saves only 10–40 ms | Low | Reasoning from architecture; no measurement |
| Launch budget table | Low | All estimates |
| `CanvasVirtualControl` is risky in WinUI 3 | Medium | Two open issues, not retested on 2.5.1 |

## Conflicts with the PRD

1. **The performance reference is misread.** The 94 ms median time-to-show for a small dialog on Windows 11 Insider builds cannot support a p95 cold first-page target. The PRD's warm-launch row says "Same", but the source gives no warm data [34].
2. **The Windows 10 target is at risk.**
   - Windows 10 22H2 Home and Pro are out of mainstream support [12].
   - .NET 10 does not list them as supported [11].
   - Mica falls back to a solid color [14].
   - The Dynamic Dependencies polyfill applies [25].
   - A third-party test measured WinUI 2.6 times slower on Windows 10 [31].
3. **The fallback aims at the wrong layer.** C++/WinRT keeps the XAML framework cost. The fallback that can help is a Win32 window that draws the first page with Direct2D or DirectComposition (estimate).
4. **"Win2D GPU page tiles" is at risk.** `CanvasVirtualControl` has two open WinUI 3 bugs [45].
5. **"Open with" on multi-select starts N processes** for packaged apps (open bug) [41].
6. **The CI gate needs more than the PRD plans for.** Failing a build at 10% over target needs at least 59 runs per mode per build and a dedicated physical runner (section 7).
7. **The open question "direct-download installer" matters for speed.** An unpackaged framework-dependent build adds bootstrap cost on Windows 10 [24]. A direct installer must be self-contained.

## What we should do

1. Ship as packaged MSIX, framework-dependent or self-contained, chosen by P2. Never ship an unpackaged framework-dependent build.
2. Use NativeAOT from day one with CsWinRT 2.3.1 and `CsWinRTAotWarningLevel=2`. Treat AOT warnings as errors. Use `x:Bind` only.
3. Build the launch window in code. Place no templated controls before the first frame. Load chrome after the first frame.
4. Start the PDFium first-page render on a worker thread in `Main`, before `Application.Start`.
5. Create the Win2D device and run all non-page work after the first frame.
6. Use a custom `Main` with single-instance redirection. Keep the process alive for N minutes after the last window closes. Measure P4.
7. Enable the `XamlOptionalChanges` performance IDs after a visual regression check.
8. Run matrix P1–P10 with the existing harness. Make the 59-run gate a CI job on a dedicated reference laptop.
9. Ask the owner to:
   - Correct the PRD's performance reference.
   - Decide whether Windows 10 is under the 400 ms gate.
   - Replace the C++/WinRT fallback with a Win32 and Direct2D first-frame fallback, decided by P6.
10. Do not use `CanvasVirtualControl` until #983 and #986 pass on 2.5.1. Prototype tiles on `CompositionVirtualDrawingSurface` instead.

## Open questions

- Does packaged activation take measurably longer than unpackaged launch?
- Are framework DLL pages often already resident because other Windows App SDK 2.x apps share the framework package (unverified)?
- Does the Run dialog stay resident between uses?
- What do Mica and `XamlControlsResources` cost at startup?
- Does Win2D 1.4.0 work on Windows App SDK 2.x?
- Is about 10 minutes of launch tests per build acceptable for CI?

## Sources

1. Windows App SDK release channels: https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/release-channels
2. Windows App SDK 2.0 release notes (stable): https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/release-notes/windows-app-sdk-2-0?pivots=stable
3. .NET support policy: https://dotnet.microsoft.com/en-us/platform/support/policy/dotnet-core
4. CsWinRT 2.3.1 release: https://github.com/microsoft/CsWinRT/releases/tag/2.3.1.260716.1
5. CsWinRT 3.0.0 preview release: https://github.com/microsoft/CsWinRT/releases/tag/3.0.0-preview.260319.2
6. Microsoft.Graphics.Win2D 1.4.0 on NuGet: https://www.nuget.org/packages/Microsoft.Graphics.Win2D/1.4.0
7. Win2D changelog: https://github.com/microsoft/Win2D/blob/winappsdk/main/CHANGELOG.md
8. Windows App SDK 1.6 release notes (Native AOT): https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/release-notes/windows-app-sdk-1-6
9. Native AOT deployment overview: https://learn.microsoft.com/en-us/dotnet/core/deploying/native-aot/
10. Windows App SDK overview: https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/
11. .NET 10 supported OS versions: https://github.com/dotnet/core/blob/main/release-notes/10.0/supported-os.md
12. Windows 10 Home and Pro lifecycle: https://learn.microsoft.com/en-us/lifecycle/products/windows-10-home-and-pro
13. Windows 10 Extended Security Updates: https://www.microsoft.com/en-us/windows/extended-security-updates
14. Mica material: https://learn.microsoft.com/en-us/windows/apps/design/style/mica
15. CsWinRT trimming and AOT support: https://github.com/microsoft/CsWinRT/blob/master/docs/aot-trimming.md
16. PowerToys Command Palette project file: https://github.com/microsoft/PowerToys/blob/main/src/modules/cmdpal/Microsoft.CmdPal.UI/Microsoft.CmdPal.UI.csproj and https://github.com/microsoft/PowerToys/blob/main/src/Common.Dotnet.AotCompatibility.props
17. WinUI AOT issues: https://github.com/microsoft/microsoft-ui-xaml/issues/10214 , /10070 , /10881 , /11149 , /11065 , /10835 , /10188
18. .NET 10 AOT hang: https://github.com/microsoft/microsoft-ui-xaml/issues/10882 and https://github.com/microsoft/CsWinRT/issues/2121
19. AOT appxsym without the exe PDB: https://github.com/microsoft/WindowsAppSDK/issues/6208
20. Win2D AOT issues: https://github.com/microsoft/Win2D/issues/960 and https://github.com/microsoft/Win2D/issues/1001
21. Best practices for your WinUI app's startup performance: https://learn.microsoft.com/en-us/windows/apps/develop/performance/app-startup-performance
22. What's new in Windows App SDK 1.6: https://blogs.windows.com/windowsdeveloper/2024/09/04/whats-new-in-windows-app-sdk-1-6/
23. Preview UWP support for .NET 9 and Native AOT: https://devblogs.microsoft.com/ifdef-windows/preview-uwp-support-for-dotnet-9-native-aot/
24. Dynamic dependencies startup cost: https://github.com/microsoft/WindowsAppSDK/issues/4697
25. Delegate Dynamic Dependencies to the OS on 24H2 and later: https://github.com/microsoft/WindowsAppSDK/pull/4949 (see also /pull/4136)
26. Deployment guide for unpackaged apps: https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/deploy-unpackaged-apps
27. Deployment guide for self-contained apps: https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/self-contained-deploy/deploy-self-contained-apps
28. WinRT API call overhead: https://github.com/microsoft/WindowsAppSDK/issues/6223
29. WinUI 3 Performance: A Leap Forward: https://github.com/microsoft/microsoft-ui-xaml/discussions/11096
30. WinUI performance optimization (WPR and WPA): https://learn.microsoft.com/en-us/windows/apps/develop/performance/winui-perf
31. WinAppLaunchCompare: https://github.com/mrlacey/WinAppLaunchCompare
32. windowsnews.ai article: https://windowsnews.ai/article/winui-3-performance-push-file-explorer-and-notepad-benchmarks-improve.418403
33. Windows Latest, Run dialog article: https://www.windowslatest.com/2026/08/31/microsoft-proves-windows-11s-modern-ui-can-beat-legacy-win32-if-it-works-hard-enough/
34. The new Run dialog: faster, cleaner, and more capable: https://devblogs.microsoft.com/commandline/the-new-run-dialog-faster-cleaner-and-more-capable/
35. Microsoft Photos September 2024 update: https://blogs.windows.com/windows-insider/2024/09/06/microsoft-photos-september-2024-update-begins-rolling-out-to-windows-insiders/
36. XamlOptionalChanges API spec: https://github.com/microsoft/microsoft-ui-xaml/pull/11110 and https://github.com/microsoft/microsoft-ui-xaml/blob/main/docs/api-specs/XamlOptionalChanges/XamlOptionalChanges-Spec.md
37. XamlChangeId values in WinUI source: https://github.com/microsoft/microsoft-ui-xaml/blob/main/dxaml/xcp/tools/XCPTypesAutoGen/XamlOM/Model/Microsoft.UI.Xaml.Settings.cs
38. Create a single-instanced WinUI app with C#: https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/applifecycle/applifecycle-single-instance
39. App instancing with the app lifecycle API: https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/applifecycle/applifecycle-instancing
40. Redirection and foreground rights: https://github.com/microsoft/WindowsAppSDK/issues/1439
41. Multi-file activation: https://github.com/microsoft/WindowsAppSDK/issues/5066 and https://github.com/microsoft/WindowsAppSDK/pull/6278
42. Edge Startup boost: https://support.microsoft.com/en-us/topic/get-help-with-startup-boost-ebef73ed-5c72-462f-8726-512782c5e442
43. Handle file activation: https://learn.microsoft.com/en-us/windows/apps/develop/launch/handle-file-activation
44. Win2D CanvasVirtualControl: https://microsoft.github.io/Win2D/WinUI3/html/T_Microsoft_Graphics_Canvas_UI_Xaml_CanvasVirtualControl.htm
45. CanvasVirtualControl bugs: https://github.com/microsoft/Win2D/issues/983 and https://github.com/microsoft/Win2D/issues/986
46. CanvasSwapChainPanel: https://microsoft.github.io/Win2D/WinUI3/html/T_Microsoft_Graphics_Canvas_UI_Xaml_CanvasSwapChainPanel.htm ; SwapChainPanel: https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.swapchainpanel
47. Improving WPF applications startup time (cold and warm definitions): https://learn.microsoft.com/en-us/archive/blogs/jgoldb/improving-wpf-applications-startup-time
48. Acquiring high-resolution time stamps: https://learn.microsoft.com/en-us/windows/win32/sysinfo/acquiring-high-resolution-time-stamps
49. CompositionTarget class (Windows App SDK): https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.media.compositiontarget
50. PresentMon: https://github.com/GameTechDev/PresentMon
51. C++/WinRT releases: https://github.com/microsoft/cppwinrt/releases
52. RAMMap: https://learn.microsoft.com/en-us/sysinternals/downloads/rammap
53. CompositionVirtualDrawingSurface: https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.composition.compositionvirtualdrawingsurface
