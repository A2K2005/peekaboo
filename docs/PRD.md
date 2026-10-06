# PRD: Preview for Windows

Oct 6, 2026 · @A2K

## Summary

Build one free, native Windows app that opens any PDF or image in under half a second and handles the everyday edits people do today across Edge, Photos, Paint and paid PDF tools.

Working name: **Preview for Windows**. Target: Windows 10 and 11, x64 and ARM64. The bar is macOS Preview: instant open, one window, no account, no ads, no upsell.

What v1 ships: view, annotate, sign, fill forms, rearrange and merge pages, crop, resize, convert, copy text from images, remove backgrounds.

## Problem

Windows has the pieces Preview has, but they are spread across four apps and none of them is fast and complete at once.

| Today on Windows | What it does well | Where it fails the user |
| --- | --- | --- |
| [Edge PDF reader](https://learn.microsoft.com/en-us/deployedge/microsoft-edge-pdf) | Read, ink, highlight, comments, form fill | Cannot rearrange pages or edit scanned PDFs ([source](https://geekchamp.com/how-to-edit-pdfs-using-microsoft-edges-built-in-pdf-editor-2/)); no XFA or JavaScript forms; opens a browser |
| [Photos](https://blogs.windows.com/windows-insider/2023/11/17/windows-photos-gets-background-remove-and-replace-along-with-other-improvements/) | Crop, rotate, background remove and replace | No PDF, no batch resize or convert, no markup on top of images |
| Paint / Snipping Tool | Quick markup | Separate app for each step |
| Free PDF suites ([PDFgear](https://www.neowin.net/software/pdfgear-desktop-free-pdf-editor-view-edit-and-convert-pdfs-no-sign-up-or-watermarks/), PDF24, PDF-XChange) | Merge, split, compress, sign | 126 MB download for PDFgear; heavy UI; some features watermark or gate behind paid tiers ([source](https://lidarmonitor.kapernikov.com/news/download-pdf-readers-editors-writers-tools-for-windows-majorgeeks)) |

Core user problem: "I got a PDF or a photo and need to do one small thing to it in the next 30 seconds." Today that means picking the right app, waiting for it, and often uploading to a website.

Second problem: iPhone photos. HEIC files need the HEIF and HEVC extensions, and HEVC can cost $0.99 in the Store ([source](https://winaero.com/how-to-open-heic-and-hevc-files-in-windows-11/?amp)).

## Goals, non-goals, success metrics

Goal: become the default app for PDFs and images on the user's PC within one week of install.

**Goals**

- Open any supported file faster than Edge, Photos or any PDF suite on the same machine.
- Cover the top 10 everyday PDF and image tasks in one window, offline.
- Feel native to Windows 11: light and dark mode, Mica, pen and touch, Explorer integration.

**Non-goals for v1**

- Editing existing PDF body text (Acrobat-style). Preview does not do this either.
- Photo library, albums, cloud sync.
- E-book, comic or Office formats.
- Mac or Linux builds.

**Success metrics** (targets are our assumptions, to validate in beta)

| Metric | Target | How measured |
| --- | --- | --- |
| Set as default PDF or image app | 50% of active installs by day 7 | Local opt-in telemetry: file association check |
| Weekly active / installs | 40% at day 30 | Opt-in telemetry |
| Task completion without leaving app | 90% of top 10 tasks in usability test | Moderated test, 15 users |
| Cold open to first page | p95 under 400 ms on reference laptop | Automated perf test per build |
| Crash-free sessions | 99.5% | Crash reporter |

## macOS Preview feature inventory

Your list covered 23 of the 44 feature groups below. Rows marked **Added** come from Apple's [Preview User Guide](https://support.apple.com/guide/preview/welcome/mac) (macOS 27 edition) and were missing from the list. Release column maps each to our plan.

| Area | Preview feature | Source | Our release |
| --- | --- | --- | --- |
| View | Sidebar with thumbnails, contact sheet, table of contents ([guide](https://support.apple.com/guide/preview/view-pdfs-and-images-prvw11470/mac)) | Added | v1 |
| View | Continuous scroll, single page, two pages side by side | Added | v1 |
| View | Zoom, actual size, zoom to selection, magnifier loupe | Added | v1 (loupe v2) |
| View | Find text in PDFs | Added | v1 |
| View | Several files in one window or tabs | Added | v1 |
| View | Full-screen slideshow of a PDF | Added | v1 |
| View | Bookmark PDF pages | Added | v2 |
| View | Step through animated GIF frames | Added | v2 |
| View | HDR image display | Added | Later |
| View | Info pane: metadata, GPS, AI-edit signs; map of where photo was taken | Yours + map Added | v1 (map v2) |
| View | Custom background color, dark mode for PDFs, toolbar customization | Yours | v1 (toolbar v2) |
| PDF | Fill forms; AutoFill from Contacts | Yours | v1 (AutoFill v2, from a saved profile) |
| PDF | Signatures from trackpad, camera, phone | Yours | v1 (mouse, pen, touch, webcam) |
| PDF | Select and copy text | Added | v1 |
| PDF | Highlight, underline, strike out | Added | v1 |
| PDF | Notes and speech bubbles | Added | v1 |
| PDF | Shapes, arrows, text boxes, freehand | Yours | v1 |
| PDF | Merge, reorder, delete, extract pages by drag | Yours | v1 |
| PDF | Insert blank page, insert image as page | Added | v1 |
| PDF | Crop and rotate pages | Added | v1 |
| PDF | Effects (grayscale, reduce file size) | Yours + size Added | v2 |
| PDF | Permanent redaction | Yours | v2 |
| PDF | Password and permission locks | Yours | v2 |
| PDF | Batch delete annotations, remove links | Yours | v2 |
| Image | Crop, resize by pixels or percent, rotate, flip | Yours | v1 |
| Image | Batch resize, rotate, convert many files | Yours | v1 |
| Image | Convert JPEG, PNG, TIFF, HEIC, PDF, WebP (OpenEXR) | Yours | v1 (OpenEXR later) |
| Image | One-click background removal | Yours | v1 |
| Image | Instant Alpha (manual color select) | Yours | v2 |
| Image | Exposure, contrast, color adjustments | Yours | v2 |
| Image | Markup on images | Yours | v1 |
| Image | Select and copy text in images (Live Text) | Yours | v1 |
| Image | New from clipboard, paste to build collages | Yours | v1 |
| Image | Apply color profile, soft-proof on another device | Added | Later |
| Import | Import from camera, scanner, phone | Added | v2 (scanner) |
| Manage | Version history and revert | Yours | v1 revert to opened; full history v2 |
| Manage | Share, drag file icon from title bar | Yours | v1 |
| Manage | Print PDFs and images | Added | v1 |
| Manage | Export PDF pages as images, images as PDF | Added | v1 |
| Manage | Lock a file against edits | Added | Later |
| Manage | Keyboard shortcuts for every tool | Added | v1 |
| AI | Writing tools: rewrite, proofread, summarize | Yours | Later (Copilot+ PCs only) |
| 3D | View and edit USD and 3D files | Yours | Not planned |
| Other | View on Apple Vision Pro | Added | Not applicable |

Apple-only items get a Windows equivalent: Contacts AutoFill becomes a saved personal profile; iPhone signature becomes pen, touch or webcam capture; dropping files on the Dock becomes Explorer multi-select and "Open with".

## Existing apps and repositories

No single Windows app matches Preview. The fast ones only view; the complete ones are heavy. Our gap is speed plus everyday editing in one window.

| Project | Type, license | Strength | Gap vs Preview | Use for us |
| --- | --- | --- | --- | --- |
| [SumatraPDF](https://en.wikipedia.org/wiki/Sumatra_PDF) | PDF reader, GPLv3 on MuPDF (AGPL) | Speed benchmark; 6.1 MB single exe vs 320 MB Adobe Reader (2017 figures) | Reader only: no page edits, minimal markup, no images | Speed and size bar to beat; study its code, do not copy (license) |
| [ImageGlass](https://github.com/d2phap/imageglass) | Image viewer, open source, .NET | 90+ formats incl. WebP, JXL, HEIC, SVG | No PDF, light editing only | Format coverage benchmark |
| [QuickLook](https://github.com/QL-Win/QuickLook) | Spacebar preview, GPL-3.0 | Instant peek from Explorer and file dialogs, plugins | Preview only, no edits | Shows demand for Mac-like flows; plan our own spacebar peek later |
| [PDF Arranger](https://github.com/Mejans/pdfarranger) | Page organizer, GPL-3.0, Python and GTK | Merge, split, rotate, crop, reorder by drag | 41 MB download, 143 MB installed on Windows ([source](https://portableapps.com/apps/office/pdf-arranger-portable)); non-native look | UX reference for page sorting |
| [PDFgear](https://www.pcworld.com/article/2105560/pdfgear-pdf-editor-review.html) | PDF suite, freeware, closed | Edit, merge, split, compress, sign, OCR, free | 126 MB download; suite-style UI | Feature reference, main free competitor |
| [Edge PDF](https://learn.microsoft.com/en-us/deployedge/microsoft-edge-pdf) | Built in | Ink, highlight, comments, forms, read aloud | No page edits; no XFA or JS forms; lives in a browser | Default we must beat on open speed |
| [Photos](https://blogs.windows.com/windows-insider/2023/11/17/windows-photos-gets-background-remove-and-replace-along-with-other-improvements/) | Built in | Background remove and replace, now on Arm64 too | No PDF, no batch | Default we must beat for quick image edits |

**Engines we can build on**

| Library | License | Verdict |
| --- | --- | --- |
| [PDFium](https://pdfa.org/wp-content/uploads/2021/06/Survey-of-OpenSource-Solutions.pdf) (Chrome's PDF engine) | Apache 2.0 | **Use.** Render, text, forms, annotations, page import and reorder. Permissive, no source-release duty. |
| [MuPDF](https://mupdf.readthedocs.io/en/1.27.0/license.html) | AGPL or paid Artifex license | **Avoid.** SumatraPDF moved to it for speed, but AGPL forces the whole app open source, or a paid license with custom pricing. |
| [qpdf](https://pdfa.org/wp-content/uploads/2021/06/Survey-of-OpenSource-Solutions.pdf) | Apache 2.0 | **Use** for encryption, permissions, file-size cleanup. PDFium cannot encrypt on save. |
| Windows Imaging Component (WIC) + Direct2D | Part of Windows | **Use** for image decode, GPU drawing, format conversion. |

Confidence: high on licenses (primary sources). Medium on PDFium's encryption gap; verify in a one-day spike.

## Scope and acceptance criteria

v1 ships the 10 tasks below. Everything else marked v2 or Later in the inventory waits for usage data.

| # | User task | Done when |
| --- | --- | --- |
| 1 | Open and read a PDF or image | Double-click in Explorer shows first page in under 400 ms p95; scroll at 60 fps on a 500-page PDF |
| 2 | Find and copy text | Ctrl+F finds text in a 500-page PDF in under 1 s; copy keeps reading order |
| 3 | Sign a document | Signature drawn once with mouse, pen or touch, saved locally, placed in 2 clicks |
| 4 | Fill a form | AcroForm fields typeable and tab-navigable; non-form PDFs accept free text boxes |
| 5 | Mark up | Highlight, underline, strike, note, arrow, shape, text box, freehand; saved as standard PDF annotations readable in Acrobat and Edge |
| 6 | Organize pages | Drag to reorder, delete, rotate, insert blank or image page; drag a thumbnail out to Explorer creates a new PDF; drop a file into the sidebar merges it |
| 7 | Crop and resize images | Crop by drag; resize by pixels or percent with aspect lock; shows resulting file size before save |
| 8 | Convert and batch | Export to JPEG, PNG, WebP, TIFF, HEIC, PDF with quality slider; apply resize, rotate or convert to 100 selected files in one action |
| 9 | Copy text from an image | Hover shows selectable text on screenshots and photos; works offline on all supported PCs |
| 10 | Remove a background | One click; result in under 3 s on a non-AI laptop; copy or save as transparent PNG |

Also in v1: tabs, slideshow, print, Windows share, light and dark themes, keyboard shortcut for every tool, undo and "revert to opened".

**v2:** redaction, passwords, reduce file size, grayscale, Instant Alpha, color adjustments, bookmarks, GIF frames, scanner import, full version history, form AutoFill profile, map for photo location.

**Later:** AI writing tools on Copilot+ PCs, spacebar quick peek from Explorer, color profiles, OpenEXR, HDR, file lock.

## Performance requirements

Speed is the product. Every build fails CI if it misses a target below by more than 10%.

Reference laptop: Intel Core i5 12th gen, 8 GB RAM, SSD, Windows 11. Low-end check: 4 GB RAM, eMMC storage. Targets are our assumptions, set from the references in the last column.

| Measure | Target | Reference |
| --- | --- | --- |
| Cold launch to first page or image | p95 under 400 ms | Microsoft's WinUI 3 Run dialog, compiled ahead of time: 94 ms median time-to-show vs 103 ms for the old Win32 one ([source](https://www.windowslatest.com/2026/08/31/microsoft-proves-windows-11s-modern-ui-can-beat-legacy-win32-if-it-works-hard-enough/)) |
| Warm launch (app already in memory) | p95 under 150 ms | Same |
| Next image in folder (arrow key) | under 50 ms for JPEG up to 24 MP | Pre-decode neighbors |
| Open 500-page, 50 MB PDF | first page under 300 ms | Render visible page only |
| Scrolling | 60 fps, no blank tiles after 100 ms | GPU tile cache |
| Memory, one 20-page PDF open | under 120 MB | Edge PDF tab is a browser process |
| Installer download | under 30 MB (excluding optional AI model) | PDFgear 126 MB, PDF Arranger 41 MB |
| Background removal, non-AI laptop | under 3 s for 12 MP photo | On-device model |

**Rules that hit these numbers**

- Compile ahead of time. No .NET JIT, no web view, no Electron.
- Draw the window before loading anything else. No network, update check or telemetry on the launch path.
- Render only visible pages at screen resolution; thumbnails on a background thread.
- Decode images at display size first, full size only on zoom or edit.
- Load OCR and background-removal models on first use, never at launch.
- Save edits incrementally; never rewrite a 50 MB PDF to add one highlight.

## UX and Windows integration

One window, content first, tools appear only when needed. Copy Preview's restraint, not its macOS look.

**Layout**

- Title bar: file name, drag handle for the file itself (drag into email or chat), tabs.
- Toolbar, 6 buttons by default: sidebar, zoom, markup, rotate, share, search. Everything else lives in the markup bar or the menu.
- Markup bar slides in under the toolbar on click or Ctrl+Shift+A. Hidden otherwise.
- Sidebar: thumbnails, contents, notes. Drag in to merge, drag out to extract.
- No home screen, no dashboard of tool tiles. Opening the app with no file shows recent files and "New from clipboard".

**Interaction rules**

- Every action has a keyboard shortcut and a right-click entry.
- Edits are non-blocking. No "Processing..." dialogs under 1 s; a slim progress bar above that.
- Pen and touch are first class: pinch zoom, two-finger rotate on images, pen draws without picking a tool.
- Autosave like Preview, with "Revert to opened" one click away. The first edit to a file asks once: overwrite or save a copy, with "Remember my choice".

**Windows integration**

| Integration | v1 behavior |
| --- | --- |
| Default app | Offer to become default for PDF, JPEG, PNG, WebP, HEIC, GIF, TIFF on first run; one click to Windows Settings |
| Explorer | "Open with" on multi-select; right-click "Convert", "Resize", "Combine into PDF" |
| Theme | Follows system light or dark, Mica background, Windows 11 rounded corners, Segoe UI Variable |
| Share | Windows share sheet |
| Print | Windows print dialog |
| HEIC | Use Windows codec if installed; otherwise bundled decoder, so iPhone photos open with no Store purchase |
| Accessibility | Narrator labels on every control, full keyboard reach, high contrast, 200% text scale |
| Language | English first; text externalized for later translation |

## Recommended tech stack

Build a native Windows app: C# on WinUI 3, compiled ahead of time (NativeAOT), with PDFium for PDFs and Windows' own imaging stack for pictures. One language, native look, no browser inside.

| Layer | Choice | Why | License |
| --- | --- | --- | --- |
| UI | WinUI 3 (Windows App SDK), C#, NativeAOT | Native Windows 11 look; Microsoft's own AOT-compiled WinUI 3 dialog beats the Win32 one on time-to-show ([source](https://www.windowslatest.com/2026/08/31/microsoft-proves-windows-11s-modern-ui-can-beat-legacy-win32-if-it-works-hard-enough/)) | MIT |
| Drawing | Win2D on Direct2D | GPU page tiles, markup, pen strokes | MIT |
| PDF | PDFium via direct C calls | Render, text, forms, annotations, page edits; same engine as Chrome | Apache 2.0 |
| PDF security and size | qpdf | Encryption, permissions, cleanup PDFium lacks | Apache 2.0 |
| Images | Windows Imaging Component | Built-in decode and encode for JPEG, PNG, TIFF, GIF, WebP; uses HEIC codec when present | Windows |
| HEIC fallback | libheif + libde265, loaded as separate DLLs | iPhone photos open without the Store codec | LGPL-3.0 (dynamic linking keeps our code closed) |
| Text in images | Windows.Media.Ocr on all PCs; Windows AI TextRecognizer on Copilot+ PCs | Offline; the AI version is faster and more accurate on Copilot+ hardware ([source](https://learn.microsoft.com/en-gb/windows/apps/windows-app-sdk/stable-channel)) | Windows |
| Background removal | Windows AI ImageForegroundExtractor on Copilot+ PCs ([source](https://learn.microsoft.com/windows/ai/overview)); elsewhere an open segmentation model on ONNX Runtime with DirectML, downloaded on first use | Works on every PC, keeps installer small | Model license to confirm in spike (MIT or Apache only) |
| Distribution | Microsoft Store (MSIX) plus winget | Auto-updates, ARM64 and x64 builds | n/a |

**Rejected**

- Electron or Tauri: a browser engine on every launch breaks the startup and memory targets.
- C++ and raw Win32: fastest possible, but an estimated 2 to 3 times the UI build effort for a gain the AOT numbers above say we do not need.
- MuPDF: AGPL, see previous section.

**Fallback:** if WinUI 3 misses the 400 ms cold-launch target in the week-2 spike, move only the launch window and page view to C++/WinRT; keep the rest.

## Risks, open questions, release plan

Biggest uncertainty is cold-launch speed on WinUI 3, so it is tested first, before any feature work.

| Risk | Impact | Mitigation |
| --- | --- | --- |
| WinUI 3 launch slower than 400 ms on low-end PCs | Core promise fails | Week-2 spike on 4 GB eMMC laptop; C++/WinRT fallback for launch path |
| PDFium form and annotation gaps (XFA forms, some signature fields) | Some government or bank PDFs fail | Detect and show "Open in Edge" for XFA; test against a 200-file corpus of real forms |
| Annotations not shown correctly in Acrobat or Edge | Users lose trust | Write only standard PDF annotation types; round-trip test in Acrobat Reader and Edge every build |
| HEVC patent exposure from bundled HEIC decoder | Legal | Legal review before v1; if blocked, prompt users to install the Windows codec instead |
| Background removal quality on non-AI PCs | Feature feels worse than Photos | Benchmark against Photos on 100 images; ship only if equal or better |
| Redaction that leaves hidden text | Data leak | Keep redaction out of v1; v2 removes underlying text and images, verified by text extraction test |

**Open questions**

- Pricing: free forever, or free core plus one-time paid pack for batch and redaction?
- Store-only, or also a direct download installer for companies that block the Store?
- Minimum OS: Windows 10 22H2 still has a large base; dropping it saves test time.

**Release plan**

1. Weeks 1 to 2: speed spike. Shell, PDFium page view, WIC image view. Gate: cold launch under 400 ms p95 on both test laptops.
2. Weeks 3 to 8: v1 tasks 1 to 6 (PDF). Gate: form corpus pass rate above 95%; annotation round-trip clean.
3. Weeks 9 to 12: v1 tasks 7 to 10 (images, OCR, background removal). Gate: background removal on par with Photos.
4. Weeks 13 to 14: closed beta, 200 users. Gate: crash-free sessions 99.5%, 15-user usability test at 90% task completion.
5. Week 15: public v1 on Microsoft Store and winget.

Timeline assumes 2 engineers and 1 designer; it is an estimate, not a commitment.

## Sources

- [Apple Preview User Guide](https://support.apple.com/guide/preview/welcome/mac) and [View PDFs and images](https://support.apple.com/guide/preview/view-pdfs-and-images-prvw11470/mac)
- [SumatraPDF, Wikipedia](https://en.wikipedia.org/wiki/Sumatra_PDF)
- [ImageGlass repo](https://github.com/d2phap/imageglass), [QuickLook repo](https://github.com/QL-Win/QuickLook), [PDF Arranger portable](https://portableapps.com/apps/office/pdf-arranger-portable)
- [MuPDF license](https://mupdf.readthedocs.io/en/1.27.0/license.html), [PDF Association open-source survey](https://pdfa.org/wp-content/uploads/2021/06/Survey-of-OpenSource-Solutions.pdf)
- [Edge PDF reader docs](https://learn.microsoft.com/en-us/deployedge/microsoft-edge-pdf), [Photos background removal](https://blogs.windows.com/windows-insider/2023/11/17/windows-photos-gets-background-remove-and-replace-along-with-other-improvements/)
- [Windows App SDK stable channel](https://learn.microsoft.com/en-gb/windows/apps/windows-app-sdk/stable-channel), [Windows AI overview](https://learn.microsoft.com/windows/ai/overview)
- [WinUI 3 Run dialog benchmark](https://www.windowslatest.com/2026/08/31/microsoft-proves-windows-11s-modern-ui-can-beat-legacy-win32-if-it-works-hard-enough/)
- [HEIC on Windows 11](https://winaero.com/how-to-open-heic-and-hevc-files-in-windows-11/?amp), [PDFgear review](https://www.pcworld.com/article/2105560/pdfgear-pdf-editor-review.html)
