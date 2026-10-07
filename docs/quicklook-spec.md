# Quick view and editor parity spec

Oct 7, 2026. Status: proposed (D21). Scope owner: @A2K.

## Goal

Select a PDF or image in Explorer, press Space, and see it at once. Press Space again and it is gone. The editor opens only on request. The target is behavior and layout parity with macOS Quick Look and Preview, with our own icons, wording, and assets, and without Apple's marks. The UI calls the peek "Quick view". Owner question: the working product name contains "Preview".

Speed: first frame under 100 ms from the key press ([NN/g 0.1 s limit](https://www.nngroup.com/articles/response-times-3-important-limits/)), full content under 400 ms ([Doherty threshold](https://lawsofux.com/doherty-threshold/)).

## 1. Interaction model

### Quick view

| Input | Behavior |
| --- | --- |
| Space in an Explorer file list or on the desktop, with a file selected | Open Quick view for the selection |
| Space or Esc in Quick view | Close. Focus returns to Explorer with the same selection. |
| Left, Right, Up, Down | Next or previous file. With 2 or more files selected, move through the selection only. With 1 file selected, move through supported siblings in Explorer view order, and move Explorer's selection to match. |
| Enter, or the **Open** button | Turn Quick view into the full editor, on the same page and zoom |
| Ctrl+Enter | Index sheet: a grid of the selected files. Click or Enter opens one. |
| Ctrl+wheel, Ctrl+= and Ctrl+-, pinch | Zoom. Ctrl+0 is actual size. |
| Wheel, Page Up, Page Down, Home, End | Scroll a multi-page PDF continuously |
| F11 | Full screen. Esc leaves full screen first. |
| Ctrl+Shift+Space (fallback hotkey) | Open Quick view for the Explorer selection when the Space hook cannot fire |

Also: an Explorer verb "Quick view" and `preview.exe --peek <path>`. Like PowerToys Peek, it never opens while the user types in rename, address, or search boxes ([Peek FileExplorerHelper](https://github.com/microsoft/PowerToys/blob/main/src/modules/peek/Peek.UI/Helpers/FileExplorerHelper.cs)).

### Quick view window

- Near-borderless: rounded corners, 1 px border, system shadow. No tabs, sidebar, status bar, or markup bar.
- Sized to content, centered on Explorer's monitor. Images show at 100% when they fit, else fit 80% of the work area. PDFs fit the page width, at most 85% of the work-area height.
- A 40 px hover strip shows the file name (centered) and, on the right: **Open**, Markup, Rotate (images), Share, Close. It fades out 1.5 s after the pointer stops.
- Markup turns the window into the editor with the markup bar open. Quick view stays read-only.

## 2. Architecture on Win32

### Processes and windows

One executable. `preview.exe --resident` starts at sign-in from `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`. It owns the hook and one hidden, pre-created window with its Direct2D device. Quick view and the editor are the same window in 2 states. **Open** shows the chrome and resizes; no new process, so page, zoom, and tiles carry over. Double-click opens reach the resident process through `integration::hand_off`, so editor launches are warm too.

### Trigger

1. A `WH_KEYBOARD_LL` hook runs on its own thread with a message loop, as the API requires ([LowLevelKeyboardProc](https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc)).
2. The callback acts only on a Space key-down with no modifiers, not injected, and not a repeat. It then checks, in order:
   - `GetForegroundWindow` class is `CabinetWClass`, `Progman`, or `WorkerW`.
   - `GetGUIThreadInfo` for that thread: `hwndFocus` class is `DirectUIHWND` or `SysListView32`, and `hwndCaret` is null.
3. On a match, it posts to the UI thread and returns 1 to swallow the key. No COM work in the hook.
4. The UI thread reads the selection: `IShellWindows`, match `IWebBrowserApp::HWND` to the foreground window, `IServiceProvider::QueryService(SID_STopLevelBrowser)`, `IShellBrowser::QueryActiveShellView`, `IFolderView2::Items(SVGIO_SELECTION)`. The desktop uses `FindWindowSW` with `SWC_DESKTOP`, then the same chain. Peek uses this chain (link above).
5. If nothing supported is selected, re-send Space with `SendInput`. The hook skips injected keys, so Explorer gets its normal Space.

### Warm state and memory

| State | Kept | Target |
| --- | --- | --- |
| Idle, never used | Hook, hidden window, D2D device | Under 25 MB private bytes (unverified, measure) |
| After a PDF | PDFium loaded, tile cache | Kept warm for 10 minutes |
| Idle 10 minutes | Drop tiles and documents, `SetProcessWorkingSetSize(-1, -1)` | Back to idle |

OCR and background removal still load on first use.

### Risks

- **Hook timeout.** On Windows 7 and later, a slow hook is removed silently, and the maximum timeout is 1,000 ms on Windows 10 1709 and later (same source). The callback does only class-name checks. Reinstall the hook every 10 minutes and on resume from sleep, because removal cannot be detected.
- **Windows 11 tabbed Explorer.** Several tabs share one `CabinetWClass` window, so the HWND match can pick a background tab. Pick the browser whose `ShellTabWindowClass` is visible. Unverified.
- **Elevated Explorer (UIPI).** COM calls into a higher-integrity Explorer fail. Then Space passes through and the fallback verb still works.
- **Games and full-screen apps.** The cost is one class-name read per Space press. Anti-cheat reaction to a low-level hook is unverified.
- **PowerToys Peek** also uses Space since 0.95 ([docs](https://learn.microsoft.com/en-us/windows/powertoys/peek)). If it runs, offer once to use our fallback hotkey only.

## 3. Learning from macOS Preview

| Preview lesson | Our editor change |
| --- | --- |
| One unified title and toolbar row ([Preview toolbar review](https://www.macworld.com/article/668356/get-the-most-out-of-mac-os-x-preview.html)) | Merge the tab strip and toolbar into one 48 px bar: file name and page count on the left, tools on the right |
| Markup toolbar hidden until the pen button or Cmd-Shift-A ([Apple](https://support.apple.com/guide/preview/prvw11470/mac)) | Keep the markup bar hidden by default; reorder its tools to match (section 4) |
| Sidebar modes: Thumbnails, Table of contents, Contact sheet, Hide ([Apple](https://support.apple.com/guide/preview/prvw11470/mac)) | Sidebar off for images and 1-page PDFs, on for multi-page PDFs, with the same 4 modes |
| Autosave and Revert To > Last Opened, no save prompts ([Apple](https://support.apple.com/guide/preview/prvw35876/mac)) | Keep autosave and **Revert to opened**. Remove every other save prompt. |
| No status bar; transient feedback only | Remove the status bar. Results show as a 2 s toast. Info bars only for decisions. |
| Several images in one window with a thumbnail sidebar | Multi-select open shows one window with the sidebar listing the files, not tabs |

## 4. Parity table

Mac behavior marked "observed" is from use, not from Apple docs, so it is unverified.

| Done | Area | macOS | Ours | Deviation and reason |
| --- | --- | --- | --- | --- |
| [ ] | Peek trigger | Space in Finder ([Apple](https://support.apple.com/guide/mac-help/mh14119/mac)) | Space in Explorer list and desktop | None |
| [ ] | Peek close | Space or close button | Space, Esc, Close | Esc added: Windows convention |
| [ ] | Next or previous | Arrow keys ([Apple](https://support.apple.com/guide/mac-help/mh14119/mac)) | Arrow keys; Explorer selection follows | None |
| [ ] | Index sheet | Cmd-Return | Ctrl+Enter | Cmd maps to Ctrl |
| [ ] | Open in app | "Open with" button | **Open** button, Enter | Enter: Explorer and Peek convention |
| [ ] | Peek zoom | Cmd-Plus, Cmd-Minus | Ctrl+=, Ctrl+-, Ctrl+wheel | None |
| [ ] | Peek actions | Markup, rotate, share, full screen | Same | None |
| [ ] | Peek opening motion | Zoom from the file icon (observed) | 120 ms fade and 0.97 to 1 scale, centered | Icon rectangles in Details view are not exposed reliably (unverified) |
| [ ] | Peek window controls | Close and full screen at top left (observed) | Close at top right | Windows convention |
| [ ] | Editor toolbar order | Sidebar, title, Info, Zoom out, Zoom in, Share, Highlight, Rotate, Markup, Search (observed) | Same order, own icons | Current "⋯" menu stays last for the rest |
| [ ] | Markup tools order | Text select, Rect select, Redact, Sketch, Draw, Shapes, Text, Highlight, Sign, Note, Shape style, Border color, Fill color, Text style, Rotate, Crop, Form fill ([Apple](https://support.apple.com/en-in/guide/preview/prvw11580/mac)) | Same order. Underline and strikethrough move into the Highlight menu. Shapes is one menu. | Redact is v2 (PRD). Remove the eraser: Delete removes the selected mark. |
| [ ] | Markup toggle | Cmd-Shift-A | Ctrl+Shift+A | None |
| [ ] | Sidebar modes | Thumbnails, TOC, Contact sheet, Hide (Option-Cmd-n, observed) | Same, Ctrl+Shift+1 to 4 | Ctrl+Alt is AltGr on many layouts |
| [ ] | Page navigation | Option-Down, Option-Up ([Apple shortcuts](https://support.apple.com/guide/preview/cpprvw0003/mac)) | Alt+Down, Alt+Up | Option maps to Alt |
| [ ] | Actual size, fit | Option-Cmd-0, Option-Cmd-9 | Ctrl+0, Ctrl+9 | Ctrl+Alt is AltGr |
| [ ] | Full screen | Control-Cmd-F | F11 | Windows convention |
| [ ] | Remove background | Shift-Cmd-K | Ctrl+Shift+K | None |
| [ ] | Inspector | Cmd-I | Ctrl+I | None |
| [ ] | Tabs | Control-Tab; tabs only when windows merge | Ctrl+Tab; tab strip only with 2+ documents | None |
| [ ] | Autosave | Saves as you work | Saves as you work; a file Preview cannot write asks where to save a copy | Owner dropped the first-edit consent on Oct 7, 2026 |
| [ ] | Versions | Browse All Versions | Revert to opened only | Windows has no per-document version store |
| [ ] | Status bar | None | None | None |
| [ ] | Window size | Fits content; last state remembered (observed) | Fits content; remember zoom, page, sidebar per file | None |

## 5. Visual spec

| Token | Light | Dark |
| --- | --- | --- |
| Canvas | `#F3F3F3` | `#1C1C1C` |
| Bar and hover strip | `#FFFFFF` at 85% | `#2B2B2B` at 85% |
| Text | `#1A1A1A` | `#FFFFFF` |
| Secondary text | `#5C5C5C` | `#C5C5C5` |
| Border | black at 8% | white at 8% |
| Accent | System accent, focus and selected tool only | Same |

Mica on Windows 11 22H2 and later, solid elsewhere (D10). High contrast keeps system colors.

| Element | Size |
| --- | --- |
| Quick view hover strip | 40 px tall, 12 px side padding, 32 px buttons, 4 px gaps |
| Quick view minimum | 360 by 240 px |
| Editor bar | 48 px, 12 px side padding, 36 px buttons, 4 px gaps, 12 px between groups |
| Markup bar | 40 px, under the editor bar |
| Sidebar | 200 px default, drag to resize |
| Toast | 32 px tall, 16 px above the bottom edge, centered |

Type: Segoe UI Variable, 14 px title, 12 px secondary.

| Motion | Duration and easing |
| --- | --- |
| Quick view open | 120 ms opacity 0 to 1 and scale 0.97 to 1, `cubic-bezier(0,0,0,1)` |
| Quick view close | 83 ms opacity 1 to 0, `cubic-bezier(1,0,1,1)` |
| Next or previous file | 83 ms crossfade, no slide |
| Quick view to editor | 167 ms window resize; the chrome fades in |
| Hover strip | 120 ms in, 167 ms out after 1.5 s idle |
| Markup bar | 167 ms slide (exists) |

Durations follow Fluent's 83, 167, and 250 ms set and its enter and exit curves ([Microsoft](https://learn.microsoft.com/en-us/windows/apps/design/motion/timing-and-easing)). When Windows animations are off, every change is instant.

**Hidden in the editor by default:** tab strip (1 document), status bar, markup bar, sidebar for images and 1-page PDFs. The toolbar stays visible in a window and auto-hides in full screen, as in Preview.

## 6. Build plan

1. **Resident process and trigger.** Create `src/resident.rs` (Run key, hook thread, fallback hotkey, hook reinstall, idle trim) and `src/selection.rs` (Explorer and desktop selection, siblings in view order, selection sync). Touch `src/main.rs` (`--resident`, `--peek`) and `src/integration.rs` (Run key and the "Quick view" verb in `register_at` and `unregister_at`).
2. **Quick view mode.** Add a peek or editor mode to `src/ui/app.rs`. Popup style, size to content, and the fade in `src/ui/window.rs`. A hover-strip-only layout in `src/ui/widgets.rs` and `src/ui/paint.rs`. Peek keys in `src/ui/commands.rs`.
3. **Handoff and index sheet.** Quick view to editor in place in `src/ui/app.rs` and `src/ui/window.rs`. The index sheet reuses the grid in `src/ui/organize.rs`. Route `integration::hand_off` opens to the resident window.
4. **Editor chrome parity.** Unified 48 px bar, toolbar order, tab strip only with 2+ documents, status bar removed, page count in the title, toasts: `src/ui/widgets.rs`, `src/ui/paint.rs`, `src/ui/app.rs`, `src/ui/infobar.rs`.
5. **Markup, sidebar, and shortcuts parity.** Tool order and menus in `src/ui/commands.rs` and `src/ui/actions.rs`. Sidebar modes and per-type defaults in `src/ui/sidebar.rs`. Shortcut map from section 4 in `src/ui/commands.rs`.
6. **Tokens, motion, and gate.** Colors and sizes in `src/ui/theme.rs`. Add a Space-to-first-frame benchmark to `src/ui/bench.rs`, with p95 under 100 ms warm, and record it in `docs/benchmarks.md`.

QuickLook ([GPL-3.0](https://github.com/QL-Win/QuickLook)) was read for its key map only. No code was copied.

## 7. Reference frames: gaps to close

Source: owner's screen recordings, read as frames on Oct 8, 2026. Sizes are at the 1280 by 720 frame scale. Frame rate is unknown, so durations are unverified. "Ours" is from screenshots of the current build. Changes use our own icons and wording.

### 7.1 Quick Look in the frames

- **Header.** One bar, about 26 px tall. Left: close and full screen as small gray circled glyphs, then the bold file name, left-aligned. Right: share, then a pill text button naming the app that opens the file.
- **Material.** The bar is translucent (frame 004). Bar and thumbnail rail share one tint from the desktop color. The bar never hides in a window.
- **Content.** The page starts below the bar, inset about 5 px, with its own rounded corners. Window corners about 10 px; soft dark shadow.
- **Rail.** A deck gets a left rail of thumbnails, about 90 px wide. The current one has a 2 px accent outline; click jumps. Text documents have no rail and show a thin scroll bar only while scrolling (frame 077).
- **Motion.** The icon grows into the window (frame 061) and the window shrinks back to it on close (frames 089, 110), in 1 to 2 frames.
- **Navigation.** Arrows swap the file in place; the file list selection follows (frames 012, 013).
- **Pointer.** I-beam over text; drag selects; right-click offers Copy (frame 144). The window stays open when another app takes focus; its selection turns gray (frame 152).
- **Size.** Centered on screen. A deck takes about 75% of the width, a portrait page about 85% of the height.

### 7.2 Preview tips

| Tip | Feature | Reached by | Looks like |
| --- | --- | --- | --- |
| 1 | Export | File > Export | Sheet: name, location, format, live file size |
| 2 | Rotate | Toolbar button, Cmd-R | "Edited" appears under the file name |
| 3 | Transform | Tools menu | Rotate Left and Right, Flip Horizontal and Vertical, Crop (Cmd-K, needs a selection), Remove Background (Shift-Cmd-K) |
| 4 | Loupe | Shapes menu | Magnifying circle; blue handle sets size, green handle sets power |
| 5 | Mask | Shapes menu | Rectangle with 8 handles; outside dims |
| 6 | Selection | First markup menu, wand button | Rectangular, elliptical, lasso, smart lasso. Wand drag paints similar colors red, then selects them |
| 7 | Sketch, draw | Pen buttons | Freehand stroke |
| 8 | Text style | Text button, Aa and color menus | Font, size, color, bold, underline, 4 alignments; swatch grid |
| 9 | Signature | Sign menu | Saved signatures and a create button; capture by trackpad, camera, or phone |
| 10 | Adjust size | Tools menu | Fit-into preset, width, height, units, resolution, keep proportions, new and old size |
| 11 | Clipboard, image text | Cmd-N; drag over image text | "Untitled" window; image text selects and copies |

Markup bar order: selection, wand, sketch, draw, shapes, text, sign, adjust color, adjust size, then shape style, border color, fill color, text style, note.

### 7.3 Gaps, ranked by impact on "instant, no barrier"

| # | Mac | Ours today | Change | Files in `src/ui` |
| --- | --- | --- | --- | --- |
| 1 | Icon grows into the window and shrinks back | 120 ms centered fade and scale | Zoom from and to the item's rectangle when Explorer reports it; fade as fallback | `quickview.rs` (`begin_show`, `dismiss`, `tick`) |
| 2 | Header always visible above the page | 40 px strip covers the page top and fades after 1.5 s (p01, p04, owner screenshot) | Reserve the bar above the document; auto-hide only in full screen. Update sections 1 and 5 | `quickview.rs` (`layout`, `strip_alpha`, `content_size`), `paint.rs` |
| 3 | Rounded, tinted, translucent window with shadow | Square corners on Windows 10 (DWM rounding is Windows 11 only); opaque margin | Mica on 11, solid tint on 10, DWM shadow. Rounding on 10 needs a composition surface (cost unverified) | `quickview.rs` (`set_style`), `window.rs`, `theme.rs` |
| 4 | Thumbnail rail for multi-page files | No Quick view sidebar (`layout.sidebar = None`), p03 | 90 px rail for PDFs of 2 or more pages, reusing editor thumbnails (Mac PDF behavior unverified) | `quickview.rs`, `sidebar.rs` |
| 5 | Left-aligned name; one button names the target app | Centered name; 6 buttons; **Open** says no destination | Left-align the name. Label "Open in *app*" when another app is the default | `quickview.rs` (`layout`, `paint`) |
| 6 | I-beam over text; right-click Copy | Pointer input reaches the document; Quick view menu unverified | Verify; add a Copy and Open menu | `actions.rs` (`context_menu`), `quickview.rs` |
| 7 | "Edited" under the file name | Page count only (q01) | Show "Edited" after a change; it confirms autosave (D22) | `widgets.rs`, `paint.rs`, `app.rs` |
| 8 | Markup bar shows its tools | Empty markup bar (q03) | Check for a mid-slide capture or a layout bug | `widgets.rs`, `paint.rs` |
| 9 | Shape style, border, fill, and text style menus | No width, color, or font controls in `MARKUP_TOOLS` | Add 4 menu buttons after the tools | `commands.rs`, `menu.rs`, `actions.rs` |
| 10 | 8 shapes, plus loupe and mask | Rectangle, ellipse, arrow | Add line, rounded rectangle, bubble, star, polygon; then loupe and mask (PDF save unverified) | `commands.rs` (`tool_group`), `document.rs` |
| 11 | Rectangular, elliptical, lasso, and wand selection | Text select, remove background | Rectangular and elliptical selection feeding Crop and Delete; lasso later | `commands.rs`, `document.rs`, `imagetools.rs` |
| 12 | Flip horizontal and vertical | Horizontal only | Add Flip vertically | `commands.rs`, `actions.rs` |
| 13 | Saved signatures list; camera capture | Draw, then Sign | List saved signatures in the Sign menu | `commands.rs`, `menu.rs` |
