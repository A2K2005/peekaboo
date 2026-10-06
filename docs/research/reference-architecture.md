# Reference architecture teardown

Date: 2026-10-06
Author: Research agent: reference architecture
Status: draft (research stopped early on owner budget; see "Open questions")

## Summary

- No project publishes a cold-launch p95. PowerToys Peek, Files, and QuickLook all hide WinUI 3 or WPF startup cost behind a resident process and a reused window [5][6][7][45][81]. Our 400 ms cold-launch target has no direct precedent in these five apps.
- SumatraPDF is the best model for PDF scheduling. It uses a LIFO queue of 8 requests, drops the oldest request, aborts renders for pages that scrolled away, and pre-renders at most 4 pages ahead, one at a time [50][51].
- Peek does not render PDFs itself. It sends `.pdf` to WebView2, which is the Edge PDF viewer [3]. QuickLook is the only project here that uses PDFium. It serializes every PDFium call behind one static lock [83].
- ImageGlass 10 is the best model for "next image < 50 ms". It prefetches outward from the current image in a spiral under a memory budget. It cancels the pass on navigation and skips the pass while the user holds an arrow key [65][67].
- NativeAOT for WinUI 3 works in production now: PowerToys CmdPal [13] and Files [34] ship it. Both then reported AOT-only bugs, including a 3 GB memory leak [19] and finalizer-backlog crashes [36]. CI must test the AOT build, not the JIT build.

## Findings

### Per-project teardown

#### 1. Microsoft PowerToys: Peek and AOT modules

| Aspect | Finding |
| --- | --- |
| License | MIT [1]. |
| Layout | `src/modules/peek/` holds 5 projects: `peek` (C++ module DLL loaded by the runner), `Peek.UI` (WinUI 3 exe), `Peek.FilePreviewer` (previewers), `Peek.Common`, and 2 UI-test projects [1][6][9]. |
| Startup | The runner calls `enable()` in `peek/dllmain.cpp`, which calls `launch_process()`. `Peek.UI` therefore starts when PowerToys starts, not on the hotkey [6]. `Peek.UI` builds a `Microsoft.Extensions.Hosting` DI host in the `App` constructor, then waits on a named event [5]. The first hotkey creates `MainWindow`. Later hotkeys reuse the window with `Hide()` and `Show()` [5][7]. |
| Threading | The UI thread owns previewers. Each navigation cancels the previous load through a new `CancellationTokenSource` [8]. Image size and metadata reads run on `Task.Run` [4]. |
| Rendering | `PreviewerFactory` tries previewers in a fixed order: image, video, audio, web browser, SQLite, archive, shell preview handler, drive, special folder, then unsupported [2]. Images use XAML `BitmapImage.SetSourceAsync` on the full file stream. The fallback is a cached shell thumbnail [4]. The `.pdf`, `.html`, `.md`, and `.svg` types go to WebView2 [3]. |
| Caching | No neighbor prefetch exists in `MainWindowViewModel` or `FilePreview` (searched for prefetch, preload, and cache) [7][8]. Archive icons have an `IconCache` [2]. |
| Packaging | Self-contained .NET plus self-contained Windows App SDK, unpackaged (`WindowsPackageType=None`), x64 and ARM64 [9][10]. The x64 installer is 282.6 MB for all modules (measured: `gh api` release assets, v0.101.2362.0) [29]. |
| AOT | `Peek.UI` imports `Common.Dotnet.AotCompatibility.props` (`IsAotCompatible=true`, `CsWinRTAotOptimizerEnabled=true`) but does not set `PublishAot` [9][10]. `PublishAot=true` is set in CmdPal (pipeline flag), PowerOCR, PowerAccent.UI, and PowerDisplay [11][12][93][94]. |
| Updates | The runner has `src/runner/UpdateUtils.cpp` and `src/common/updating/` (unverified: behavior not read). |
| Tests | Legacy UI tests use Appium.WebDriver with WinAppDriver and MSTest [27]. The new `UITestAutomation.Next` harness drives `winapp.exe` (winappcli, pinned to v0.3.2 in CI) and adds screenshots, screen recording, and image-hash visual asserts [26]. Peek UI tests are functional only, with 30 s to 60 s timeouts [91]. Startup speed is measured in the field: Peek logs `HotKeyToVisibleTimeMs` telemetry per open [7]. PR #38008 added open-time telemetry "to evaluate the benefits of applying AOT" [28]. No perf gate in CI was found (searched the tree for perf and benchmark). |

**What PowerToys changed for NativeAOT**

- Removed `PublishReadyToRun` and `PublishTrimmed` once AOT was the default [13].
- Replaced the Markdown control with the Labs `MarkdownTextBlock` [14]. Made Adaptive Card rendering AOT-safe (PR #40134, title only).
- Replaced `rd.xml` with preservation attributes [15].
- Added `partial` to classes that CsWinRT and the MVVM Toolkit generate code for [18].
- Converted `[ObservableProperty]` fields to partial properties, moved JSON to source generators, and moved COM interop to `ComWrappers` [17].
- Set `EventSourceSupport=true` so telemetry fires in AOT builds [11].
- Early blockers were extension loading over COM and the AllApps provider [16].

#### 2. Files (files-community/Files)

| Aspect | Finding |
| --- | --- |
| License | MIT [30]. The repo also contains `LICENSE-MPL` (MPL 2.0); which files it covers is unverified. |
| Layout | `src/Files.App` (WinUI 3), `Files.App.Controls`, `Files.App.Storage`, `Files.App.CsWin32` (interop), `Files.App.Server` (out-of-proc WinRT server), `Files.App.Launcher` (C++), `Files.Core.SourceGenerator`; tests in `tests/` [31]. |
| Startup | Custom `Main` with `DISABLE_XAML_GENERATED_MAIN`. It redirects activation to a running instance with `AppInstance.FindOrRegisterForKey` and `RedirectActivationTo` [40][32]. `OnLaunched` builds the DI container on a background task while the UI thread constructs `MainWindow`. It reads the theme and backdrop settings off the UI thread, because the first frame needs them [38]. A second task warms commands at `BelowNormal` thread priority [38]. Under NativeAOT the splash screen is skipped; the code comment says AOT "startup is fast enough" [38]. After activation, services start in staggered `Task.WhenAll` groups "to reduce resource contention". The update check runs on a background task [39]. |
| Threading | UI thread plus thread-pool tasks. A known bug: blocking the UI thread in the background state stalled about 98,000 WinRT finalizers and crashed `ComWrappers` under NativeAOT [36]. |
| Caching | Shell thumbnails through `FileThumbnailHelper`, using shell cache buckets [41-a] (limits unverified). When idle in the background, Files calls `K32EmptyWorkingSet` [38]. |
| Packaging | MSIX bundle for x86, x64, and ARM64. Distributed through the Store and as a sideload `.appinstaller` from `cdn.files.community` [32][47]. Release builds publish with `PublishAot=true` and `SelfContained=true` [33]. `DirectPInvoke` binds system DLLs at link time [32][35]. |
| Updates | The Store update service or the sideload update service. Re-checks every 5 h (stable) or 2 h (preview) while running [39]. |
| Tests | The CI installs the MSIX and runs `Files.InteractionTests` with Appium 3, `appium-windows-driver` 6, MSTest, and `Axe.Windows` accessibility asserts, with up to 3 retries [41][42]. AGENTS.md says the repo lacks "a suitable set of tests for AI agents" [31]. No perf tests in CI (searched `ci.yml`). Crashes go to Sentry [38]. |
| AOT | NativeAOT enabled in PR #18842 (198 files, +1204/-506) with CsWinRT level-3 AOT warnings to block regressions [34]. Code-review rules reject `DllImport`, `ComImport`, and runtime reflection [31]. |

#### 3. SumatraPDF (GPLv3, patterns only)

| Aspect | Finding |
| --- | --- |
| License | GPL-3.0 [49]. MuPDF is AGPL (PRD). Nothing below is copied. |
| Layout | Flat `src/` (C++): `SumatraPDF.cpp` (WinMain), `RenderCache.*`, `DisplayModel.*`, `Engine*.cpp`, `UpdateCheck.cpp`; vendored libraries in `ext/` (MuPDF, dav1d, libjpeg-turbo) [49]. |
| Startup | WinMain runs about 30 cheap steps before any window: crash handler, flags, policies, common controls, DPI from the cursor's monitor, render cache, dark mode, settings, language [53]. It then finds an existing instance and forwards files to it [53]. Session restore creates the window hidden, loads only the selected tab when lazy loading is on, then shows the window [53]. Deferred until after the window: the automatic update check, stale-file cleanup, and missing-file pruning [53]. All libraries are delay-loaded [53]. Release builds exit without freeing memory ("fast exit") [53]. |
| Threading | One UI thread. Render threads spawn lazily, up to max(8, core count), capped at 32. A semaphore wakes one thread per queued request [50][51]. Results return to the UI thread through `uitask::Post` [54]. |
| Rendering | Pages split into tiles no larger than the screen. On allocation failure the tile size halves, down to a 200 px minimum, and the cache is dropped [51]. See the scheduling question below. |
| Caching | Fixed array of 128 bitmaps. Eviction order: invisible pages of the same document, then the oldest entry of other documents [50][51]. The source says the limit "should be based on amount of memory taken" (TODO) [50]. MuPDF display lists are cached per page [52]. |
| Packaging | One exe that is also the installer. A portable mode extracts `libsumatrapdf.dll` to `%LOCALAPPDATA%\SumatraPDF-data\<build-id>\` [53][57]. Releases are not hosted on GitHub (measured: `gh api` latest release has 0 assets). Current installer size is unverified. |
| Updates | Background HTTP check at most once a day, after the window exists; results are posted to the UI thread [54]. Pre-release builds check on every start [54]. |
| Tests | Unit tests compiled into the debug exe (`-unit-tests`). A daily CI job builds with ASan and runs GUI-driving tests from `tests/run-github-ci.ts` with a 120 min timeout [55]. Perf: a `-bench` flag plus `render-benchmark.py` (manual, 10 runs per file) and an opt-in `PerfLog` [56][53]. |

#### 4. ImageGlass 10 (GPLv3 Classic, patterns only)

| Aspect | Finding |
| --- | --- |
| License | Source is GPLv3 ("ImageGlass Classic"). Pro binaries and the Store package ship under separate paid terms. An MIT source grant is available on request [61][62]. The PRD lists it only as "open source"; treat it as GPL. |
| Layout | v10 lives in `source/`: `ImageGlass.Lib` (Avalonia, shared), `ImageGlass.Win32`, `ImageGlass.Mac`, `ImageGlass.Linux`. v9 (WinForms) lives in `v9/` [62]. |
| Stack | Avalonia 12.1.3, SkiaSharp 3.119.4, Magick.NET 14.17.1 (Q16-HDRI), .NET 10, `PublishAot=True`, self-contained, trimmed, `DisableRuntimeMarshalling=true` [63][64]. |
| Startup | `Main` records `StartupTrace` marks, sets up single-instance handling and platform services, then starts Avalonia [71][70]. `StartupTrace` is an opt-in milestone profiler from process start to first paint, enabled by a CLI flag [70]. v9 had a "Startup Boost" (launch at sign-in); users must disable it before upgrading to v10 [73]. |
| Decode | `CodecRegistry` registers SVG, then Skia, then Magick.NET as a content-sniffing fallback. Plugins can outrank built-ins by priority. A per-extension cache remembers the winning codec [68]. Holding an arrow key sets `ShouldLoadFullResolution=false` in Turbo mode (the default) [67][66]. Large images get mipmap tiles loaded by one background worker over a 4096 px proxy [74]. |
| Caching | See the prefetch question below. Disk thumbnail cache with an MB budget and LRU eviction by last-access time [69]. |
| Packaging | MSIX 44.3 MB, ZIP 41.8 MB, MSI 33.7 MB (Pro Business only) for win-x64 (measured: `gh api` release assets, 10.0.6.906) [77][73]. |
| Updates | Background check every 7 days plus up to 24 h of random jitter [72]. |
| Tests | No test projects and no CI workflows in the repo (measured: repo tree has no `test` paths; `.github` has only templates). |

#### 5. QuickLook (GPL-3.0, patterns only)

| Aspect | Finding |
| --- | --- |
| License | GPL-3.0 [78]. |
| Layout | `QuickLook` (WPF host, .NET Framework 4.6.2), `QuickLook.Common` (plugin contract), `QuickLook.Native` (Explorer integration), and `QuickLook.Plugin.*` (about 30 viewers) [78][85]. |
| Startup | Resident tray app with a single-instance mutex. A second launch forwards the path over a named pipe [82]. At start it loads every plugin and calls `Init()` [82]. The update check waits 120 s and runs at most once every 30 days [82]. First launch extracts resources and takes "a few seconds" [88]. |
| Plugin model | `IViewer` has `Priority`, `Init`, `CanHandle`, `Prepare`, `View`, and `Cleanup` [79]. `Prepare` must be fast; it sets the window size before show. The window then shows with a busy state. `View` runs at `DispatcherPriority.Input` and clears the busy flag when done [79][80]. |
| Instant show | After a window closes, the manager creates the next `ViewerWindow` at once, so a window is always ready [81]. |
| PDF | PDFium through `PdfiumViewer.Updated` 2.14.5 and the `bblanchon.PDFiumV8.Win32` binaries [85]. Every render runs inside one static `lock` [83]. The whole current page renders as one bitmap at screen DPI; there are no tiles [83][86]. The wrapper copies the whole PDF into a `MemoryStream` [84]. It reopens the document every 50 thumbnails and forces `GC.Collect` 500 ms after each page render [83][86]. |
| Packaging | EXE 61.7 MB, MSI 90 MB, APPX 130.3 MB, ZIP 116.8 MB (measured: `gh api` release assets, 4.5.0) [90]. |
| Tests | None found in the tree (unverified: not searched in depth). |

### Comparison table

| | PowerToys Peek | Files | SumatraPDF | ImageGlass 10 | QuickLook |
| --- | --- | --- | --- | --- | --- |
| License | MIT [1] | MIT (+MPL file) [30] | GPLv3 [49] | GPLv3 Classic [61] | GPLv3 [78] |
| UI | WinUI 3 [9] | WinUI 3 [32] | Win32 C++ [49] | Avalonia + Skia [63] | WPF [85] |
| Compile | JIT, AOT-compatible [9] | NativeAOT [33][34] | Native C++ | NativeAOT [63] | .NET Framework JIT [85] |
| PDF | WebView2 [3] | n/a | MuPDF [52] | n/a | PDFium, global lock [83] |
| Images | XAML `BitmapImage` + WIC [4] | Shell thumbnails [41-a] | MuPDF engines [49] | Skia, then Magick.NET [68] | Own plugin [78] |
| Resident process | Yes, starts with PowerToys [6] | Optional, "leave running" [45] | No (reuses running instance) [53] | v9 Startup Boost [73] | Yes, tray [82] |
| Prefetch | None found [8] | n/a | ≤ 4 pages, chained [50] | Spiral, MB budget [65] | None found |
| Cache limit | n/a | Working-set trim [38] | 128 bitmaps [50] | 1000 MB, 100 MB file, 8000 px [66] | None found |
| Packaging | WiX EXE, 282.6 MB all modules [29] | MSIX, Store + appinstaller [47] | Self-installing EXE, portable [53] | MSIX 44 MB / ZIP / MSI [77] | EXE 62 MB / MSI / APPX [90] |
| Update | Runner (unverified) | Store or sideload, 5 h [39] | Daily, after window [54] | 7 d + jitter [72] | 30 d, 120 s delay [82] |
| UI tests | winappcli + legacy WinAppDriver [26][27] | Appium + Axe.Windows [41][42] | GUI tests + ASan, daily [55] | None | None found |
| Perf measurement | Field telemetry [7] | None found | `-bench`, PerfLog (manual) [56] | Opt-in StartupTrace [70] | None found |

### Q1. Startup techniques, and which apply to WinUI 3 + NativeAOT

| Technique | Who uses it | Applies to us? |
| --- | --- | --- |
| Single instance; forward args to the running process | Files [40], SumatraPDF [53], QuickLook [82], ImageGlass [71] | Yes. `AppInstance.FindOrRegisterForKey` is in Windows App SDK [40]. This is how warm launch < 150 ms is reachable. |
| Keep a hidden window and reuse it (hide or cloak, not destroy) | Peek [7], QuickLook [81], CmdPal cloak [21] | Yes. CmdPal cloaks instead of `SW_HIDE` because XAML shows "a few frames of flicker" on `SW_SHOW` [21]. |
| Resident or pre-launched process | Peek [6], QuickLook [82], Files option [45], ImageGlass v9 [73] | Only as an opt-in. It would not count toward cold-launch p95. |
| Build services and settings off the UI thread, in parallel with window construction | Files [38] | Yes. |
| Defer non-first-frame work: updates, telemetry, warm-ups at low priority | Files [38][39], SumatraPDF [53][54], QuickLook [82] | Yes. This matches the PRD rule "no network on the launch path". |
| Lazy-load restored tabs; only the visible one loads | SumatraPDF [53] | Yes, for v1 tabs. |
| Delay-load native DLLs; bind system DLLs at link time | SumatraPDF delayload [53]; Files `DirectPInvoke` [35] | Yes. `DirectPInvoke` is a NativeAOT feature [35]. |
| Skip the splash screen when AOT makes startup fast | Files [38] | Yes. Show content, not a splash. |
| Fast exit (let the OS free memory) | SumatraPDF [53] | Probably. Measure first. |
| Startup milestone tracing | ImageGlass [70], SumatraPDF PerfLog [53] | Yes. Needed for the CI perf gate. |

### Q2. SumatraPDF render scheduling, memory bound, and engine thread safety

1. **Visible first.** The queue holds 8 requests. Threads take the newest request first (LIFO). When the queue is full, the oldest request is dropped and its callback reports "aborted" [50][51].
2. **Skip and cancel on scroll.** A thread skips a dequeued request if the page is no longer visible or near-visible. A new request aborts any in-flight render for a page that is no longer near-visible, through an abort cookie that MuPDF polls [51].
3. **Prefetch.** After a visible page finishes, at most 4 more pages render one at a time as a chain. The chain stops when its anchor page leaves the view [50].
4. **First paint.** At the first zoom level a page renders as one tile, or as 2 tiles of the top row when split in 4 [51].
5. **Memory bound.** 128 cached bitmaps maximum [50]. Tiles never exceed screen size, and the tile size halves on allocation failure [51]. `FreeNotVisible` drops off-screen entries [51]. The count-based limit is a known weakness: one large page can cost as much as many small ones [50].
6. **Non-thread-safe engine.** MuPDF gets lock callbacks, one cloned `fz_context` per thread, and 3 mutexes: document, page list, and page render [52]. Each page's display list is built once under the lock. The list is then replayed without a lock on any thread, because running a display list is concurrency-safe [52]. PDFium has no display-list equivalent (unverified). With PDFium, expect one engine thread (see Conflicts, item 3).

### Q3. How Peek hosts PDF and image previewers on WinUI 3

- One `FilePreview` control hosts one `IPreviewer` at a time. `PreviewerFactory` picks it by extension in a fixed order [2][8].
- PDF: `WebBrowserPreviewer` navigates WebView2 to the file URI. PDF rendering is therefore the Edge PDF viewer [3]. A 2022 issue asks to load WebView2 only on demand to cut memory [24].
- Images: `ImagePreviewer` reads the size first (`GetPreviewSizeAsync`) so the window can be sized. It then loads the full file into a XAML `BitmapImage`, with a cached shell thumbnail as the fallback [4].
- Each navigation disposes the previewer and creates a new one. This causes a white flash; PR #49963 (open) adds previewer reuse [22].
- PowerToys' separate Explorer PDF preview handler uses `Windows.Data.Pdf`. It renders at most 10 pages at control width and blocks on `GetAwaiter().GetResult()` [25].

### Q4. How Files and PowerToys test WinUI 3 UI and measure performance

- **Files:** CI builds the MSIX and installs it with a self-signed certificate on a GitHub-hosted runner at 1920 x 1080. It runs Appium 3 with `appium-windows-driver` (WinAppDriver), MSTest, and `Axe.Windows` accessibility asserts, with 3 retries [41][42]. No perf measurement in CI. Users report startup times in issues: 45 s in 4.2.34, about 3 s after a fix [48]; 12 s to restore many tabs [44].
- **PowerToys:** Moving from WinAppDriver and Appium [27] to winappcli UI Automation, with screenshots, screen recording, and image-hash asserts [26]. Speed is measured in the field with telemetry (`HotKeyToVisibleTimeMs`) [7][28], not in CI.
- Neither project gates builds on performance. The PRD's per-build perf gate has no template in these repos.

### Q5. Reported startup and memory regressions

| Project | Item | What happened |
| --- | --- | --- |
| PowerToys | #48834 [19] | CmdPal grew to about 3 GB and caused input lag. Repro on the Release/AOT build only, not on Debug. Three leaks, one in thread-pool cleanup. |
| PowerToys | #49787 [23] | Peek is slower than Photos on first open. An elevated-to-unelevated launch path adds seconds. |
| PowerToys | #40598 [20] | "Standing up a XAML hwnd takes a long time"; asks to pay that cost once. A maintainer says the memory cost is weighed against launch speed. |
| PowerToys | #22478 [24], #34026 [92] | Peek memory (bitmaps, WebView2); Settings app slow to start. |
| Files | #18950 [36] | NativeAOT crash from about 98,000 pending finalizers while the UI thread was blocked in the background. |
| Files | #18874 [37] | AOT-only crash: a `ReadOnlyDictionary` value collection cannot bind to `ItemsSource`. |
| Files | #18955 [48], #14996 [44], #4180 [43] | 45 s startup regression; slow multi-tab restore; long-running perf tracking issue. |
| SumatraPDF | #5849 [57] | 1 s startup delay when the DLL loads from the data folder instead of the exe folder. |
| SumatraPDF | #5568 [58] | Startup slows as the settings file grows to 20,000 lines (1000 recent-file states). |
| SumatraPDF | #4803 [59], #5530 [60] | Restoring many tabs delays the new file; the window shows before the file loads and appears "Not responding". |
| ImageGlass | #2388 [75], #2408 [76] | VRAM leak from creating a new GPU image per tile per frame; slow gallery on large network folders. |
| QuickLook | #342 [87], #1521 [88], #1669 [89] | Closing the window does not stop a large-PDF load, so memory fills up; first start takes seconds; PDF preview 5x slower after an update. |

## Confidence

| Finding | Level | Reason |
| --- | --- | --- |
| SumatraPDF queue, abort, prefetch, and cache limits | High | Read in current `master` source [50][51][52]. |
| Peek uses WebView2 for PDF, resident process, window reuse | High | Read in source [3][5][6][7]. |
| ImageGlass spiral prefetch and defaults | High | Read in source [65][66][67]. |
| Files and PowerToys ship NativeAOT WinUI 3 apps | High | csproj, pubxml, and merged PRs [11][13][33][34]. |
| AOT-only regressions exist | High for the listed items; medium that they generalize | Issues state AOT-only repro [19][37][36]. |
| No project gates CI on performance | Medium | Searched trees and workflows; a private Microsoft pipeline could exist. |
| PDFium is not thread-safe | Medium | Inferred from QuickLook's global lock [83]; primary PDFium docs not read (unverified). |
| WinUI 3 cold start is slow without a resident process | Medium | Maintainer statements [20][43]; no numbers published. |

## Conflicts with the PRD

1. **Cold launch p95 < 400 ms.** None of the 5 apps publishes a cold-start number. Two WinUI 3 apps (Peek, Files) rely on resident processes, and their maintainers call XAML window creation slow [20][43]. Files skips its splash screen under AOT but gives no numbers [38][46]. The week-2 spike is the only real evidence source. Keep the C++/WinRT fallback.
2. **Installer < 30 MB.** ImageGlass 10, which is NativeAOT, trimmed, and self-contained with Skia and Magick.NET, ships at 33.7 MB to 44.3 MB [77]. A self-contained Windows App SDK plus PDFium, qpdf, and libheif may exceed 30 MB (unverified). A Store MSIX that depends on the shared Windows App Runtime package avoids bundling it (unverified size gain).
3. **PDF engine threading versus "thumbnails on a background thread" and 60 fps scroll.** QuickLook serializes all PDFium calls behind one lock [83]. If PDFium is not thread-safe (unverified), thumbnails and page tiles share one engine thread. SumatraPDF's parallel render threads depend on MuPDF display lists [52], which we cannot use (AGPL).
4. **NativeAOT stability.** The PRD treats AOT as a pure speed win. Evidence shows AOT-only leaks and crashes [19][36][37]. Budget time for AOT-specific testing.
5. **Memory < 120 MB for a 20-page PDF.** A count-based cache like SumatraPDF's (128 screen-size tiles) can exceed 120 MB on its own [50]. Copying the file into memory, as QuickLook does, adds the full file size [84]. Neither is compatible with the target; we need a byte budget.

## What we should do

1. **Single instance and window reuse (warm < 150 ms).** Use `AppInstance.FindOrRegisterForKey` and `RedirectActivationTo` [40]. On close of the last window, cloak it instead of destroying it [21], and trim the working set when idle [38]. Measure how long to keep the process alive; do not make it start at sign-in.
2. **Parallel, minimal startup (cold < 400 ms).** Build settings and services on a background task while the UI thread builds the window [38]. Do not use `Microsoft.Extensions.Hosting` on the launch path; Peek does [5] (its cost is unmeasured). No splash screen [38]. Start the update check, crash reporter, and warm-ups only after the first page is drawn [39][54].
3. **Startup tracing from day 1 (CI perf gate).** Add milestone marks from process start to first paint, enabled by a flag, like ImageGlass `StartupTrace` [70]. The CI harness launches the AOT build N times and fails on p95. No reference project does this in CI; we must build it.
4. **PDF scheduler (first page < 300 ms, 60 fps).** Copy SumatraPDF's pattern, not its code: a small LIFO queue, drop the oldest, abort renders for pages that left the view, and prefetch at most 4 pages one at a time [50][51]. Run one PDFium thread that owns every PDFium call; the UI thread never calls PDFium. Verify in the spike whether PDFium supports render abort.
5. **Tiles and fallbacks (60 fps, no blank tiles after 100 ms).** Cap tile size at screen size and halve it on allocation failure [51]. Draw a stale or low-resolution tile under a missing tile [74]. Create GPU bitmaps once per tile, not per frame [75].
6. **Byte-budget caches (120 MB).** Use a byte budget, not an entry count [50]. Evict invisible pages first [51]. Open PDFs from the file, not from a full in-memory copy [84]. Cancel all work for a document when it closes [87].
7. **Image prefetch (next image < 50 ms).** Prefetch outward from the current image under an MB budget. Cancel on navigation; skip while a key repeats and show a lower-resolution image instead; decode full size on key-up [65][67]. Reuse the image control between images to avoid the white flash [22]. Decode at display size, not full size as Peek does [4].
8. **NativeAOT hygiene (all launch targets).** Use the PowerToys AOT props pattern [10]: CsWin32 or `LibraryImport`, no `DllImport` or `ComImport` [31], source-generated JSON [17], MVVM partial properties [17], attributes instead of `rd.xml` [15], and `DirectPInvoke` for system DLLs [35]. Run UI tests on the AOT build, because bugs show only there [19][37].
9. **UI and accessibility tests.** Use Appium with `appium-windows-driver` and `Axe.Windows`, as Files does [41][42], or winappcli as PowerToys does [26]. This covers the PRD's Narrator and keyboard requirements.
10. **Installer < 30 MB.** Track the installer size on every build, as the perf targets are tracked. Avoid general-purpose codec bundles like Magick.NET [64]. Rely on WIC first, as the PRD already plans.
11. **Avoid:**
    - WebView2 for PDFs [3].
    - Blocking UI-thread waits (`GetAwaiter().GetResult()`) [25][36].
    - Timed `GC.Collect` calls [86].
    - Unbounded recent-file state in settings [58].
    - Loading native DLLs from outside the app folder [57].

## Open questions

Not checked because the owner's usage budget ran out:

1. PDFium thread safety and render-abort support: read `fpdfview.h` and the PDFium docs.
2. Size of a minimal self-contained WinUI 3 NativeAOT app, compared with one that depends on the Windows App Runtime package.
3. Measured cold-start numbers for any NativeAOT WinUI 3 app. CmdPal and Files publish none [46].
4. PowerToys updater behavior (`src/runner/UpdateUtils.cpp`, not read).
5. Files thumbnail cache limits (`FileThumbnailHelper.cs` only skimmed) and the scope of `LICENSE-MPL`.
6. Peek `ShellPreviewHandlerPreviewer.cs`: the fetch failed, not read.
7. Whether XAML `BitmapImage` decodes to render size when `DecodePixelWidth` is not set.
8. The ImageGlass 10 announcement post (HTTP 403 behind a bot check) and whether v10 keeps Startup Boost.
9. SumatraPDF current installer size and QuickLook's Explorer hook (`QuickLook.Native`).
10. QuickLook test coverage, which was not searched in depth.

## Sources

1. PowerToys repository, https://github.com/microsoft/PowerToys
2. Peek PreviewerFactory.cs, https://github.com/microsoft/PowerToys/blob/main/src/modules/peek/Peek.FilePreviewer/Previewers/PreviewerFactory.cs
3. Peek WebBrowserPreviewer.cs, https://github.com/microsoft/PowerToys/blob/main/src/modules/peek/Peek.FilePreviewer/Previewers/WebBrowserPreviewer/WebBrowserPreviewer.cs
4. Peek ImagePreviewer.cs, https://github.com/microsoft/PowerToys/blob/main/src/modules/peek/Peek.FilePreviewer/Previewers/MediaPreviewer/ImagePreviewer.cs
5. Peek App.xaml.cs, https://github.com/microsoft/PowerToys/blob/main/src/modules/peek/Peek.UI/PeekXAML/App.xaml.cs
6. Peek module dllmain.cpp, https://github.com/microsoft/PowerToys/blob/main/src/modules/peek/peek/dllmain.cpp
7. Peek MainWindow.xaml.cs, https://github.com/microsoft/PowerToys/blob/main/src/modules/peek/Peek.UI/PeekXAML/MainWindow.xaml.cs
8. Peek FilePreview.xaml.cs, https://github.com/microsoft/PowerToys/blob/main/src/modules/peek/Peek.FilePreviewer/FilePreview.xaml.cs
9. Peek.UI.csproj, https://github.com/microsoft/PowerToys/blob/main/src/modules/peek/Peek.UI/Peek.UI.csproj
10. Common.Dotnet.AotCompatibility.props and Common.SelfContained.props, https://github.com/microsoft/PowerToys/blob/main/src/Common.Dotnet.AotCompatibility.props
11. CmdPal UI csproj, https://github.com/microsoft/PowerToys/blob/main/src/modules/cmdpal/Microsoft.CmdPal.UI/Microsoft.CmdPal.UI.csproj
12. PowerAccent.UI.csproj, https://github.com/microsoft/PowerToys/blob/main/src/modules/poweraccent/PowerAccent.UI/PowerAccent.UI.csproj
13. PR #41350 "[CmdPal] Enable AOT by default", https://github.com/microsoft/PowerToys/pull/41350
14. PR #40551 "[AOT] Enable AOT for CmdPal", https://github.com/microsoft/PowerToys/pull/40551
15. PR #41031 "[AOT] Remove rd.xml from CmdPal", https://github.com/microsoft/PowerToys/pull/41031
16. Issue #38279 "Build with .NET AOT", https://github.com/microsoft/PowerToys/issues/38279
17. Issue #49091 "[Workspaces] Enable AOT compatibility", https://github.com/microsoft/PowerToys/issues/49091
18. PR #36194 "Resolve AOT Build Error in Peek.UI", https://github.com/microsoft/PowerToys/pull/36194
19. Issue #48834 "Command Palette RAM leak", https://github.com/microsoft/PowerToys/issues/48834
20. Issue #40598 "Peek should keep a cloak / uncloak its HWND", https://github.com/microsoft/PowerToys/issues/40598
21. PR #39170 "CmdPal: Cloak the window instead of hiding it", https://github.com/microsoft/PowerToys/pull/39170
22. PR #49963 "[Peek] Enable reusable previewers", https://github.com/microsoft/PowerToys/pull/49963
23. Issue #49787 "Peek takes longer to open than Photos", https://github.com/microsoft/PowerToys/issues/49787
24. Issue #22478 "[Peek] Minimize memory usage", https://github.com/microsoft/PowerToys/issues/22478
25. PdfPreviewHandlerControl.cs, https://github.com/microsoft/PowerToys/blob/main/src/modules/previewpane/PdfPreviewHandler/PdfPreviewHandlerControl.cs
26. UITestAutomation.Next FRAMEWORK-PARITY-PLAN.md and WinappCli.cs, https://github.com/microsoft/PowerToys/blob/main/src/common/UITestAutomation.Next/FRAMEWORK-PARITY-PLAN.md
27. UITestAutomation.csproj, https://github.com/microsoft/PowerToys/blob/main/src/common/UITestAutomation/UITestAutomation.csproj
28. PR #38008 "[AOT] add some module editor open time telemetry", https://github.com/microsoft/PowerToys/pull/38008
29. PowerToys release v0.101.2362.0, https://github.com/microsoft/PowerToys/releases/tag/v0.101.2362.0
30. Files repository, https://github.com/files-community/Files
31. Files AGENTS.md, https://github.com/files-community/Files/blob/main/AGENTS.md
32. Files.App.csproj, https://github.com/files-community/Files/blob/main/src/Files.App/Files.App.csproj
33. Files win-x64.pubxml, https://github.com/files-community/Files/blob/main/src/Files.App/Properties/PublishProfiles/win-x64.pubxml
34. PR #18842 "Feature: Enabled NativeAOT", https://github.com/files-community/Files/pull/18842
35. PR #18857 "Enable Direct P/Invoke", https://github.com/files-community/Files/pull/18857
36. PR #18950 "Fixed crashes caused by the blocked UI thread", https://github.com/files-community/Files/pull/18950
37. PR #18874 "Fixed the most frequent crashes in the preview release", https://github.com/files-community/Files/pull/18874
38. Files App.xaml.cs, https://github.com/files-community/Files/blob/main/src/Files.App/App.xaml.cs
39. Files AppLifecycleHelper.cs, https://github.com/files-community/Files/blob/main/src/Files.App/Helpers/Application/AppLifecycleHelper.cs
40. Files Program.cs, https://github.com/files-community/Files/blob/main/src/Files.App/Program.cs
41. Files ci.yml, https://github.com/files-community/Files/blob/main/.github/workflows/ci.yml
41-a. Files FileThumbnailHelper.cs, https://github.com/files-community/Files/blob/main/src/Files.App/Utils/Storage/Helpers/FileThumbnailHelper.cs
42. Files.InteractionTests.csproj, https://github.com/files-community/Files/blob/main/tests/Files.InteractionTests/Files.InteractionTests.csproj
43. Issue #4180 "Improving Performance", https://github.com/files-community/Files/issues/4180
44. Issue #14996 "Improve perceived startup time with multiple tabs", https://github.com/files-community/Files/issues/14996
45. PR #13236 "Keep Files running in the background for quicker startup", https://github.com/files-community/Files/pull/13236
46. Files blog "Announcing Files v4.2.10", https://files.community/blog/posts/v4-2-10
47. Files cd-sideload-stable.yml, https://github.com/files-community/Files/blob/main/.github/workflows/cd-sideload-stable.yml
48. Issue #18955 "Startup Time extremely long", https://github.com/files-community/Files/issues/18955
49. SumatraPDF repository, https://github.com/sumatrapdfreader/sumatrapdf
50. SumatraPDF RenderCache.h, https://github.com/sumatrapdfreader/sumatrapdf/blob/master/src/RenderCache.h
51. SumatraPDF RenderCache.cpp, https://github.com/sumatrapdfreader/sumatrapdf/blob/master/src/RenderCache.cpp
52. SumatraPDF EngineMupdf.cpp, https://github.com/sumatrapdfreader/sumatrapdf/blob/master/src/EngineMupdf.cpp
53. SumatraPDF SumatraPDF.cpp (WinMain), https://github.com/sumatrapdfreader/sumatrapdf/blob/master/src/SumatraPDF.cpp
54. SumatraPDF UpdateCheck.cpp, https://github.com/sumatrapdfreader/sumatrapdf/blob/master/src/UpdateCheck.cpp
55. SumatraPDF windows-daily.yml, https://github.com/sumatrapdfreader/sumatrapdf/blob/master/.github/workflows/windows-daily.yml
56. SumatraPDF render-benchmark.py, https://github.com/sumatrapdfreader/sumatrapdf/blob/master/cmd/scripts/render-benchmark.py
57. SumatraPDF issue #5849, https://github.com/sumatrapdfreader/sumatrapdf/issues/5849
58. SumatraPDF issue #5568, https://github.com/sumatrapdfreader/sumatrapdf/issues/5568
59. SumatraPDF issue #4803, https://github.com/sumatrapdfreader/sumatrapdf/issues/4803
60. SumatraPDF issue #5530, https://github.com/sumatrapdfreader/sumatrapdf/issues/5530
61. ImageGlass LICENSE, https://github.com/d2phap/ImageGlass/blob/develop/LICENSE
62. ImageGlass README, https://github.com/d2phap/ImageGlass/blob/develop/README.md
63. ImageGlass.Win32.csproj, https://github.com/d2phap/ImageGlass/blob/develop/source/ImageGlass.Win32/ImageGlass.Win32.csproj
64. ImageGlass Directory.Packages.props, https://github.com/d2phap/ImageGlass/blob/develop/source/Directory.Packages.props
65. ImageGlass PhotoManager_Caching.cs, https://github.com/d2phap/ImageGlass/blob/develop/source/ImageGlass.Lib/Common/Photoing/Manager/PhotoManager_Caching.cs
66. ImageGlass Config.cs, https://github.com/d2phap/ImageGlass/blob/develop/source/ImageGlass.Lib/Settings/Config.cs
67. ImageGlass AppAPIProvider_Hotkeys.cs, https://github.com/d2phap/ImageGlass/blob/develop/source/ImageGlass.Lib/Common/ServiceProviders/AppAPIs/AppAPIProvider_Hotkeys.cs
68. ImageGlass CodecRegistry.cs, https://github.com/d2phap/ImageGlass/blob/develop/source/ImageGlass.Lib/Common/Photoing/Codecs/Registry/CodecRegistry.cs
69. ImageGlass ThumbnailDiskCache.cs, https://github.com/d2phap/ImageGlass/blob/develop/source/ImageGlass.Lib/Common/Photoing/ThumbnailDiskCache.cs
70. ImageGlass StartupTrace.cs, https://github.com/d2phap/ImageGlass/blob/develop/source/ImageGlass.Lib/Common/Loggers/StartupTrace.cs
71. ImageGlass Win32 Program.cs, https://github.com/d2phap/ImageGlass/blob/develop/source/ImageGlass.Win32/Program.cs
72. ImageGlass UpdateConstants.cs and UpdateProvider.cs, https://github.com/d2phap/ImageGlass/blob/develop/source/ImageGlass.Lib/Common/ServiceProviders/Update/UpdateConstants.cs
73. ImageGlass release 10.0.4.819 notes, https://github.com/d2phap/ImageGlass/releases/tag/10.0.4.819
74. ImageGlass PR #2394 "Load mipmap tiles asynchronously", https://github.com/d2phap/ImageGlass/pull/2394
75. ImageGlass PR #2388 "Fix VRAM leak when panning large images", https://github.com/d2phap/ImageGlass/pull/2388
76. ImageGlass PR #2408 "Reduce gallery I/O", https://github.com/d2phap/ImageGlass/pull/2408
77. ImageGlass release 10.0.6.906, https://github.com/d2phap/ImageGlass/releases/tag/10.0.6.906
78. QuickLook repository, https://github.com/QL-Win/QuickLook
79. QuickLook IViewer.cs, https://github.com/QL-Win/QuickLook/blob/master/QuickLook.Common/Plugin/IViewer.cs
80. QuickLook ViewerWindow.Actions.cs, https://github.com/QL-Win/QuickLook/blob/master/QuickLook/ViewerWindow.Actions.cs
81. QuickLook ViewWindowManager.cs, https://github.com/QL-Win/QuickLook/blob/master/QuickLook/ViewWindowManager.cs
82. QuickLook App.xaml.cs, https://github.com/QL-Win/QuickLook/blob/master/QuickLook/App.xaml.cs
83. QuickLook PdfPageExtension.cs, https://github.com/QL-Win/QuickLook/blob/master/QuickLook.Plugin/QuickLook.Plugin.PDFViewer/PdfPageExtension.cs
84. QuickLook PdfDocumentWrapper.cs, https://github.com/QL-Win/QuickLook/blob/master/QuickLook.Plugin/QuickLook.Plugin.PDFViewer/PdfDocumentWrapper.cs
85. QuickLook PDF viewer csproj, https://github.com/QL-Win/QuickLook/blob/master/QuickLook.Plugin/QuickLook.Plugin.PDFViewer/QuickLook.Plugin.PdfViewer.csproj
86. QuickLook PdfViewerControl.xaml.cs, https://github.com/QL-Win/QuickLook/blob/master/QuickLook.Plugin/QuickLook.Plugin.PDFViewer/PdfViewerControl.xaml.cs
87. QuickLook issue #342, https://github.com/QL-Win/QuickLook/issues/342
88. QuickLook issue #1521, https://github.com/QL-Win/QuickLook/issues/1521
89. QuickLook issue #1669, https://github.com/QL-Win/QuickLook/issues/1669
90. QuickLook release 4.5.0, https://github.com/QL-Win/QuickLook/releases/tag/4.5.0
91. Peek PeekFilePreviewTests.cs (UITests.Next), https://github.com/microsoft/PowerToys/blob/main/src/modules/peek/Peek.UITests.Next/PeekFilePreviewTests.cs
92. PowerToys issue #34026 "Performance tune Settings app for startup time", https://github.com/microsoft/PowerToys/issues/34026
93. PowerOCR.csproj, https://github.com/microsoft/PowerToys/blob/main/src/modules/PowerOCR/PowerOCR/PowerOCR.csproj
94. PowerDisplay.csproj, https://github.com/microsoft/PowerToys/blob/main/src/modules/powerdisplay/PowerDisplay/PowerDisplay.csproj
