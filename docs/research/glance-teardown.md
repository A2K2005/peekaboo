# Glance teardown

Date: 2026-10-07. Author: Research agent: glance teardown. Status: complete. Items marked "unverified" need a test before we rely on them.

Subject: [RedtRocks/glance](https://github.com/RedtRocks/glance), shallow clone at commit `78e091c` (2026-10-07), app version 0.6.4, Apache-2.0. All glance paths below are relative to that clone. No glance code, text, or assets were copied into this repo. Ideas are described in our own words.

## Summary

- Glance is a broad, shipping Tauri 2 app (Rust 6,776 lines, TypeScript 25,244 lines) that covers 6 of our 10 v1 tasks in full and 4 in part. It is on the Microsoft Store and has a web version. Its headless tests pass: 81 of 81 Rust, 22 of 22 thumbnailer, 10 of 10 MCP bridge, and 280 of 282 TypeScript (the 2 failures are website-only).
- Its strongest areas are Windows shell integration, release engineering, and data-safety UX (version history, external-change detection). Its installer and CI check registry values that most apps never test.
- Its weakest areas are performance evidence and doc accuracy. It publishes no launch time. Its README claims a 35 KB gzipped startup bundle. My build of the same commit loads about 461 KB gzipped at startup, including pdf-lib, which the README says loads only on first edit.
- Security has one high-impact open question. The `glance://` file server returns any local file with `Access-Control-Allow-Origin: *`, and Tauri registers it on every webview, including the AI company websites shown in the sidebar. Exploitability is unverified.
- For us, the 3 best lessons are: keep Explorer thumbnails when we become the default app, show a one-time default-app offer, and keep the opened file in a local store before the first overwrite. Our plan is stronger on speed method, rendering, write safety, privacy, and high contrast.

## Method

1. Read every Rust module in `src-tauri/src`, the ADRs in `docs/adr`, the CI and release workflows, the NSIS hooks, and the TypeScript modules on the open, render, save, and UI paths.
2. Ran headless tests only. `npm ci --ignore-scripts` installed `node_modules` inside the clone. `cargo test` ran with its target folder in the session scratchpad. The main crate needs a `dist/index.html` placeholder (glance's own Linux CI does the same). I removed `dist/`, `public/pdfjs/`, and `src-tauri/gen/` after the runs. `git status` in the clone shows no tracked changes.
3. Built the frontend once (`npx vite build`) into the scratchpad to measure the startup bundle.
4. Compared our UI from code and from wave 1 screenshots in `pfw-worktrees/w1-a/artifacts/screenshots/`. I did not launch either app (D15).

## Findings

### 1. What glance is and what it claims

| Claim | Source | Checked |
| --- | --- | --- |
| Free, open-source Preview alternative for Windows 10 and 11, x64 and ARM64 | `README.md` | Release workflow builds x64 and ARM64 NSIS installers (`.github/workflows/release.yml`). |
| 190+ file types: PDF, images incl. HEIC and camera RAW, 3D models, Office previews, text, email, e-books | `README.md`, `docs/FORMATS.md` | Extension lists in `src-tauri/src/decode/formats.rs` and `src-tauri/tauri.conf.json`. |
| Installer under 20 MB, about 18 MB for x64 | `README.md`, `site/index.html` | Not built. Other docs say 10 MB (`docs/adr/0001-tauri-webview2.md`) and 6 MB (`docs/adr/0002-bundled-subject-model.md`). |
| Startup bundle about 35 KB gzipped; PDF.js on first PDF, pdf-lib on first edit | `README.md` "Why it's light" | False at this commit. See section 4. |
| Nothing you open leaves your PC; two optional network calls | `PRIVACY.md` | True in code, with caveats in section 3. |
| AI apps can use it through MCP; Ask AI sidebar for 8 AI companies | `docs/AI-APPS.md`, ADR 0014, ADR 0015 | Code present (`src-tauri/src/mcp/`, `src-tauri/src/agents.rs`). |

Target users are Windows users who want Preview's everyday PDF and image edits. `CONTEXT.md` is a domain glossary (Document, Page, Markup, Redaction, Version). The README marks the app "early preview".

### 2. Feature coverage against our PRD

| # | PRD task | Glance | Evidence | Gap against our acceptance bar |
| --- | --- | --- | --- | --- |
| 1 | Open and read | Done | `src/ui/views/PdfView.tsx`, `src/ui/views/PdfPage.tsx`, `src/pdf/thumbs.ts`, `src-tauri/src/protocol.rs` | No launch or scroll measurement exists. |
| 2 | Find and copy text | Partial | `src/pdf/search.ts` | Search returns page plus snippet, no hit rectangles. Text extraction runs page by page on first search. Copy order is PDF.js text-layer order. No 500-page timing. |
| 3 | Sign | Done | `src/ui/markup/SignatureDialog.tsx`, `src-tauri/src/signatures.rs` | Mouse, pen with pressure, camera, photo import; stored with DPAPI. Two-click placement unverified. |
| 4 | Fill a form | Done | `src/ui/views/PdfPage.tsx` (PDF.js form layer), `src/state/documents.ts` (`saveDocument`) | Text boxes on non-form PDFs come from markup. |
| 5 | Mark up | Done | `src/core/annotations.ts`, `tests/annotations.test.ts` | Standard subtypes with appearance streams, including arrows as Line annotations. Tested with PDF.js only, not Acrobat or Edge. |
| 6 | Organize pages | Done | `src/core/pageOps.ts`, `src/ui/useFileDrop.ts`, `src/ui/sidebar/pageDrag.ts` | Drag out uses `tauri-plugin-drag`. File drop on the page list inserts at the drop point. |
| 7 | Crop and resize | Partial | `src/ui/image/AdjustSizeDialog.tsx`, `src/core/image/size.ts` | The size shown is uncompressed (width × height × 4), not the file size after save. |
| 8 | Convert and batch | Partial | `src-tauri/src/encode.rs`, `src/state/batch.ts` | WebP export is lossless only. No HEIC export. JPEG has a quality slider. No 100-file timing. |
| 9 | Copy text from an image | Partial | `src/state/ocrActions.ts`, `src-tauri/src/ocr.rs` | A command copies all recognized text. No hover selection on images. PDFs get an invisible OCR text layer. |
| 10 | Remove a background | Done | `src-tauri/src/subject.rs`, ADR 0002 | One click. The model sees a 320 × 320 copy, so 12 MP edges are coarse. ADR reports 1.1 s inference on a 4-core VM; no 12 MP end-to-end number. |

| Also in v1 | Glance | Evidence |
| --- | --- | --- |
| Tabs | Done | `src/ui/TabStrip.tsx` |
| Slideshow | Done | `src/ui/views/Slideshow.tsx` |
| Print | Done | `src/state/print.ts`: every page rasterized at 150 DPI as JPEG, then the WebView2 print dialog |
| Windows share | Done | `src-tauri/src/shell.rs` (`share_files`) |
| Light and dark themes | Done | `src/styles/app.css` tokens. No app styles for forced colors (high contrast). |
| Shortcut for every tool | Done | `src/state/commands.ts` (67 bindings), rebindable in Settings |
| Undo and revert to opened | Done, different UX | Undo per document; "revert" is File > Browse Versions (`src/ui/dialogs/VersionsDialog.tsx`) |

Glance also ships features we list as v2 or later: redaction, version history, Instant Alpha, color adjustments, passwords, reduce file size, scanner import, bookmarks, certificate signatures, and a PDF thumbnail handler for Explorer.

### 3. Architecture

**Split.** Rust (`src-tauri/src`) does file access, image decode and encode, OCR, background removal, certificate checks, version history, shell integration, and AI process control. TypeScript with Preact (`src/`) does the UI, all PDF work (PDF.js for rendering, pdf-lib for writing), image editing on canvases, markup, redaction, and the MCP tools.

**IPC.** `src-tauri/src/lib.rs` registers 71 commands. Large data avoids JSON: binary request bodies with values in headers (`write_file`, `save_image`, `ocr_image`, `subject_mask`), and a custom `glance://` scheme (`src-tauri/src/protocol.rs`) with 3 routes: `/file` (raw bytes), `/decode` (decoded pixels as 32-bit BMP, optional downscale), and `/archive` (one CBZ entry).

**PDF engine.** PDF.js legacy build, loaded on first PDF (`src/pdf/engine.ts`). pdf-lib (`@cantoo/pdf-lib`) writes markup, page edits, OCR text, redaction, and encryption. Markup saves rewrite the whole file (`src/state/actions.ts` `serialize`, `src/core/annotations.ts`). Only certificate signing uses an incremental update (`src/core/pdfSign.ts`).

**Image decode and encode.** WIC first, with decode-time downscale through `IWICBitmapScaler` and EXIF orientation (`src-tauri/src/decode/wic.rs`). Pure-Rust fallbacks: `image`, `jxl-oxide`, `hayro-jpeg2000`, PSD, ICNS, embedded RAW previews (`src-tauri/src/decode/mod.rs`). Every decode runs inside `catch_unwind`, so a decoder panic fails one file, not the app. Encode uses the `image` crate with optional ICC profiles (`src-tauri/src/encode.rs`). Browser-native formats go to WebView2 as raw bytes.

**OCR.** `Windows.Media.Ocr` with the user's languages, capped at `OcrEngine.MaxImageDimension` (`src-tauri/src/ocr.rs`). Same API as ours.

**Background removal.** U²-Net-p (4.5 MB ONNX) embedded with `include_bytes!` and run by `tract-onnx` with multithreaded matrix multiply (`src-tauri/src/subject.rs`). No ONNX Runtime DLL. The model hash in `src-tauri/models/README.md` matches the file (`309c8469…ddd8`, checked).

**File associations and shell.** Per-user NSIS install. `src-tauri/windows/hooks.nsh` registers Default apps capabilities, Send To, and classic Explorer verbs. It also copies the previous default app's thumbnail and preview handlers to the extension key, and restores `PerceivedType` that the per-user key hides. CI installs the app and asserts these registry values, then uninstalls and asserts cleanup (`.github/workflows/ci.yml`). Explorer verb launches arrive one per file; `src-tauri/src/explorer.rs` merges launches that arrive within 600 ms. A separate COM DLL renders PDF thumbnails with `Windows.Data.Pdf` (`src-tauri/thumbnailer/src/lib.rs`).

**Packaging and updates.** NSIS installers for x64 and ARM64, SHA256SUMS, winget submission, and an MSIX bundle for the Store (`packaging/README.md`). The Store signs MSIX, so the Store channel needs no purchased certificate. GitHub installers are unsigned. Updates: a daily GitHub release check that only notifies (`src/state/updates.ts`); the Store build skips it.

**Network calls.**

| Call | When | Default | Evidence |
| --- | --- | --- | --- |
| `api.github.com` latest release | 5 s after launch, at most daily | On (GitHub build) | `src/ui/App.tsx`, `src/state/updates.ts` |
| `cloud.umami.is` install count | 5 s after launch, at most daily | On (both builds) | `src/state/installCount.ts`, ADR 0016 |
| AI company websites, agents via `npx`, Antigravity zip from `dl.google.com` | Only when the user opens Ask AI | Off until used | `src-tauri/src/website.rs`, `src-tauri/src/agents.rs` |
| WebView2 bootstrapper | Install time, if WebView2 is missing | n/a | `tauri.conf.json` `webviewInstallMode` |

No call runs before the first frame, but 2 run by default within 5 s of every launch. Our PRD forbids update checks and telemetry on the launch path and makes telemetry opt-in.

**Other launch work.** `setup` in `src-tauri/src/lib.rs` starts a thread that loads the background-removal model, binds a loopback TCP listener for MCP, and writes an endpoint file. The window starts hidden and shows after the first UI render.

### 4. Performance

**Startup path.** Process start, WebView2 environment, `index.html` from embedded assets, parse the entry JavaScript, Preact render, show the window, restore the last session, then open files. Opening a PDF reads the whole file through `/file` (`std::fs::read`), then passes a copy to the PDF.js worker (`bytes.slice()` in `src/pdf/engine.ts`).

**Measured startup bundle** (my `vite build` of `78e091c`, Vite 7.3.6 from the lockfile):

| File loaded by `index.html` | Bytes | Gzipped |
| --- | --- | --- |
| Entry chunk `index-*.js` | 581,091 | 192,443 |
| `pdflib-*.js` (statically imported by the entry) | 580,627 | 249,529 |
| `index-*.css` | 91,018 | 18,886 |
| Total | 1,252,736 | 460,858 |

The README claims about 35 KB gzipped. WebView2 reads embedded assets, so gzip size matters less than the 1.16 MB of JavaScript it must parse. No test guards this number.

**Rendering and caching.** Each PDF page is one canvas, rendered when an `IntersectionObserver` with a 100% margin reports it visible, and freed when it leaves (`src/ui/views/PdfPage.tsx`). A page canvas is capped at 16 MP, so deep zoom renders below screen resolution. There are no tiles. Thumbnails use a 2-job FIFO queue and an unbounded cache per document revision (`src/pdf/thumbs.ts`). Visible pages have no priority over thumbnails and no cancel when scrolled away.

**Large files.** The whole PDF stays in memory in at least 2 copies (document bytes and the worker copy). Decoded non-browser images cross as full BMPs (96 MB for 24 MP). The Explorer thumbnailer skips files over 512 MB. Search extracts all page text sequentially on first use.

**Published numbers.** Only ADR 0002: 1.9 s single-threaded and 1.1 s multithreaded U²-Net-p inference on a 4-core CI VM. No script or test reproduces it. No launch, scroll, memory, or search numbers exist.

### 5. Quality

**Tests.** I ran them headless on this PC.

| Suite | Result | Notes |
| --- | --- | --- |
| `src-tauri` `cargo test` | 81 of 81 passed | Includes 3 WIC tests and a real model inference test |
| `src-tauri/thumbnailer` | 22 passed, 1 ignored | Ignored test needs an installed handler |
| `src-tauri/mcp-bridge` | 10 of 10 passed | |
| `npx vitest run` (40 files) | 280 of 282 passed | Both failures are in `tests/sitePages.test.ts` (website). One is CRLF from a Windows checkout (`core.autocrlf=true`). The other is a SyntaxError importing `scripts/indexnow.mjs`; cause unverified (Node 25 here, Node 22 in CI). |

Coverage is strong on pure logic: autosave timing as a state machine with a fake clock (`tests/autosave.test.ts`, 14 tests), page operations, markup round trips, redaction verified by PDF.js text extraction (`tests/redact.test.ts`), shortcuts, i18n (fails on UI text that skips translation). There are no UI component tests, no rendering tests, no performance tests, and no Acrobat or Edge round trip.

**CI.** Every pull request runs typecheck, tests, the web build, Windows Rust tests, the NSIS installer with install and uninstall registry checks, an MCP smoke test against the installed app, a test-signed MSIX install, an ARM64 `cargo check`, and Linux Rust tests (`.github/workflows/ci.yml`). The release workflow fails when the tag does not match the app version or the release notes lack a section for it (`.github/workflows/release.yml`).

**Error handling.** Messages are specific and actionable, for example the missing OCR language message in `src-tauri/src/ocr.rs`. Decoders are panic-isolated. Weak points:

- `write_atomic` (`src-tauri/src/commands.rs`) and `save_image` (`src-tauri/src/encode.rs`) rename a temp file over the target with no `sync_all`. A power loss after the rename can leave an empty file.
- The temp name is fixed (`<name>.<ext>.glance-tmp`), so 2 concurrent writes to one file collide.
- `beforeOverwrite` (`src/state/versions.ts`) logs a warning and still overwrites when it cannot save the original version.

**Security.**

| Area | Finding | Evidence |
| --- | --- | --- |
| Capabilities | Narrow plugin permissions; `opener` limited to `ms-settings:` URLs | `src-tauri/capabilities/default.json` |
| CSP | `script-src 'self' 'wasm-unsafe-eval'`; `connect-src` allows only GitHub and Umami off-device | `src-tauri/tauri.conf.json` |
| HTML previews | DOMPurify, forms and frames stripped, remote images replaced by links, email in a frame sandboxed without scripts | `src/preview/sanitize.ts`, `src/preview/email.ts` |
| Local file server | `/file?path=` returns any readable file with `Access-Control-Allow-Origin: *`. The handler ignores which webview asks. Tauri 2.11.6 registers app URI schemes on every webview, including the remote AI sites added as child webviews. A remote page could read local files if WebView2 lets it fetch `http://glance.localhost`. Unverified, high impact. | `src-tauri/src/protocol.rs`, `src-tauri/src/website.rs`, Tauri `src/manager/webview.rs` lines 228 to 242 |
| Child webviews | Navigation allows any `https` URL, so the exposure is not limited to the 8 company sites | `src-tauri/src/website.rs` `on_navigation` |
| File writes | `write_file` writes any path the webview names. Acceptable only while the webview stays free of script injection. | `src-tauri/src/commands.rs` |
| Process launch | The module says the webview never passes a command line, but `agent_login` accepts extra arguments and environment variables from the webview | `src-tauri/src/agents.rs` |
| Downloads | Antigravity zip downloaded with `curl` over HTTPS and executed, with no pinned hash. Agents start through `npx -y` with no version pin. | `src-tauri/src/agents.rs` |
| MCP endpoint | Loopback only, 40-hex token, constant-time compare, 5 s handshake timeout | `src-tauri/src/mcp/mod.rs`, `src-tauri/src/mcp/endpoint.rs` |
| Secrets | API keys in Windows Credential Manager; signatures with DPAPI | `src-tauri/src/agents/secrets.rs`, `src-tauri/src/signatures.rs` |
| Command injection | Ghostscript gets arguments as an argument list with `-dSAFER`, not a shell string | `src-tauri/src/decode/eps.rs` |
| Overwrite safety | Autosave writes back to the original with no first-edit prompt (ADR 0004). A file-stamp check pauses autosave when another app changed the file. MCP tools never overwrite without an explicit flag (ADR 0014). | `docs/adr/0004-autosave-with-explicit-save.md`, `src/state/versions.ts` |

**Accessibility.** 103 `aria-label`s, 61 `role` attributes, live regions for toasts and search counts, and `prefers-reduced-motion` handling (`src/styles/app.css`). The app's own CSS has no `forced-colors` rules; the only ones come from PDF.js's annotation-layer CSS. Only 2 `:focus-visible` rules exist. No UI Automation testing is visible.

**Doc drift.** These docs disagree with the code:

- ADR 0008 says PDF autosave appends incremental updates. The code rewrites the file whenever markup exists.
- `history.rs` cites ADR 0005 for version history; the right ADR is 0006.
- ADR 0006 says old versions are purged when the disk is low, with no fixed quota. The code uses a fixed 2 GB limit.
- Installer size appears as 6 MB, 10 MB, and 18 MB in 3 places.

### 6. Dependencies and licenses

| Item | License | Risk |
| --- | --- | --- |
| `libheif-js` 1.23.2 (libheif with libde265 compiled to WASM) | LGPL-3.0 | High. Used only by the web version, but the desktop frontend build also emits its 1.99 MB chunk, which Tauri embeds in the app. That ships an HEVC decoder (patent-sensitive) and LGPL code inside the executable. The string `de265` appears 21 times in the bundle. |
| U²-Net-p model | Code Apache-2.0; trained on DUTS-TR | Medium. DUTS-TR is released for research use with no explicit license, the same class of training-data risk that made us reject BiRefNet, BEN2, and ormbg (D16). |
| Ghostscript | AGPL | Low. Not bundled. Runs as a separate program only if the user installed it. |
| `jszip` | MIT or GPL-3.0 | Low. MIT can be chosen. |
| `mtx-decompressor` (via `@aiden0z/pptx-renderer`), `dompurify` | MPL-2.0; MPL-2.0 or Apache-2.0 | Low. File-level copyleft only. |
| Rust crates (`src-tauri/Cargo.lock`) | MIT, Apache-2.0, BSD, Zlib, MPL-2.0 (5), Unicode-3.0 | Low. No GPL-only, AGPL, or non-commercial crate. |
| Third-party notices | None in the desktop app | Medium. Apache-2.0 and MIT need notices in the distribution. |

## Where glance is better than ours today

Ranked by impact on the PRD goals: default app within 7 days, and 90% task completion in usability tests. "Ours now" comes from code on branch `v1` and wave 1 screenshots, not from a running app.

| Rank | Area | Glance evidence | Ours now | Change we could make |
| --- | --- | --- | --- | --- |
| 1 | Explorer thumbnails survive becoming the default app | `src-tauri/windows/hooks.nsh` copies the old default's thumbnail and preview handler IDs to the extension key and restores `PerceivedType`; `.github/workflows/ci.yml` asserts both | `src/integration.rs` `register_at` writes ProgIDs and `DefaultIcon` only. No handler copy, no test. If PDF thumbnails turn into our icon, users may switch the default back. | When we register, copy any thumbnail and preview handler found under the previous default ProgID to `HKCU\Software\Classes\.<ext>\ShellEx` when the extension has none. Record what we wrote for unregister. Add a test that reads `HKCR\.jpg` `PerceivedType` and `.pdf` `ShellEx` after register. |
| 2 | Default-app offer | `src/state/defaultApp.ts`, `src/ui/DefaultAppBar.tsx`, `src-tauri/src/shell.rs`: a bar shown once, 3 s after launch, only when not default for PDF and images; the button opens the app's own Default apps page; status re-checked on window focus | `src/integration.rs` can open Default apps settings, but nothing in `src/ui` calls it. No status check. | After the first file opens, show a dismissible bar with one button. Check status with `AssocQueryStringW` for `.pdf`, `.jpg`, and `.png`. Re-check on `WM_ACTIVATE`. Ask once per install. |
| 3 | Tool discovery | Visible labeled menu bar with type-specific menus (Pages for PDFs, Image for images), visible Search box, page field "1 of 4", Customize Toolbar (`assets/screenshots/markup-light.webp`, `src/core/menus.ts`) | 6 icon-only buttons and a "⋯" menu. A markup bar of 15 icon-only tools, 3 of them greyed with no reason (`light-document.png`). Tooltips and key tips help. | Show text labels on the 6 toolbar buttons when the window is 1,000 px wide or more. Group markup tools with dividers. Hide tools that do not apply to the file type instead of greying them. Compare both in the usability test. |
| 4 | Feedback for results, progress, and errors | `src/ui/Toasts.tsx` (status and alert roles, live region), `withBusy` labeled spinner (`src/state/ui.ts`), `src/ui/InfoBar.tsx` for states that need a decision | Every message goes to one status line (`src/ui/app.rs`, `Event::Progress` and `Event::Finished` set `state.status`). Errors are easy to miss and are not announced. | Add a toast layer for results ("Saved", "Copied 42 words") and an info bar for decisions. Expose both as UI Automation live regions through AccessKit. Keep the status line for page info. |
| 5 | External changes and a safety copy | `src/state/versions.ts` (`checkDisk`, `beforeOverwrite`), `src/ui/ConflictBar.tsx`: stamp check before every save; autosave pauses; Reload or Keep my version | Not in `src` yet (W2-3 in progress) | Store size and modified time at open. Compare before each write. On a mismatch, pause autosave and ask. Copy the opened bytes to local app data before the first overwrite, and refuse to overwrite if that copy fails. |
| 6 | Page organizing | Contact sheet grid with a size slider and a one-line hint (`assets/screenshots/pages-light.webp`), insertion line, drag pages between tabs by hovering a tab, file drop inserts at the drop point (`src/ui/useFileDrop.ts`) | Thumbnail list with keyboard navigation (`src/ui/sidebar.rs`). `WM_DROPFILES` opens files only and gives no hover feedback (`src/ui/window.rs`). W2-5 not started. | Replace `DragAcceptFiles` with an OLE `IDropTarget` to get the hover point. Draw an insertion line in the sidebar. Show a one-line hint the first time the sidebar opens. |
| 7 | Empty state | `src/ui/views/Welcome.tsx`: says files can be dropped anywhere, a merge tip, a drag-over highlight | Heading, Open button, and a recent files list (`light-empty.png`). Recent files are better than glance. No "New from clipboard" (PRD), no drop hint, no drag-over highlight. | Add "New from clipboard", enabled only when the clipboard holds an image. Add "or drop files here". Highlight on drag-over (needs the `IDropTarget` from rank 6). |
| 8 | Settings | `src/ui/dialogs/SettingsDialog.tsx`: theme override, autosave toggle with a plain explanation, reopen tabs, update and count toggles, dark PDFs, rebindable shortcuts with conflict warnings, default-app status | No settings UI | A small Settings sheet with 3 items: save behavior (ask, overwrite, copy), so "Remember my choice" can be undone; theme (system, light, dark); default-app status with a button. |
| 9 | Dark appearance for PDFs | Setting and "⋯" menu item that inverts pages in dark mode (`SettingsDialog.tsx`) | None (no `invert` in `src`). The PRD inventory lists PDF dark mode as v1. | Apply a Direct2D color matrix to page tiles when the theme is dark and the setting is on. |
| 10 | Visual polish | File-type icons on tabs, page shadow, icon sidebar tabs, Fluent stroke and layer tokens (`src/styles/app.css`) | Pages sit flat on the grey canvas; tabs show names only; sidebar tabs are text (`light-document.png`) | Add a 1 px stroke and a soft shadow under pages. Add a type glyph from Segoe Fluent Icons to each tab. |
| 11 | Release engineering | CI on every pull request with installer and registry checks; release checks for version, tag, and notes; SHA256SUMS; winget and Store automation | No CI. Strong local scripts and a perf gate. | A script, local or in CI, that builds, runs tests serially, runs register and unregister into a test key, and asserts the values. |
| 12 | User-facing docs | `PRIVACY.md`, `docs/FORMATS.md`, `packaging/store/listing.md`, per-format website pages | `THIRD-PARTY-NOTICES.md` and strong internal docs. No privacy policy or store listing. | Write `PRIVACY.md` before beta. The Store asks for a privacy policy URL, and the PRD plans opt-in telemetry. |
| 13 | Keyboard extras | Rebindable shortcuts, single-letter tool keys, Space to pan (`src/core/shortcuts.ts`, README) | Fixed, tested table with no Ctrl+Alt chords; key tips on the toolbar (`src/ui/commands.rs`) | Add Space-drag to pan, and single-letter tool keys only while the markup bar has focus. |

Features glance has that are v2 or later for us: redaction, version history, Instant Alpha, color adjustments, certificate signature checks, Explorer PDF thumbnails, and 190+ formats. They do not change v1 scope.

Where our UI is already stronger: a high-contrast theme that follows the system (`src/ui/theme.rs`), UI Automation through AccessKit, key tips, 225% text-scale screenshots, Windows animation settings honored (`src/ui/app.rs`), and recent files on the empty state.

## Lessons for us

### Ideas worth adopting

1. **Keep Explorer's thumbnails and preview panes when we register.** Copy missing handler IDs from the previous default to the extension key, restore `PerceivedType`, record what we wrote, and assert it in a test. Helps the default-app goal and the "Default app" and "Explorer" rows of the PRD integration table (W1-D, W2-7).
2. **Make the default-app offer a status-driven bar.** Check the real association, ask once, open the app's own page in Settings, and re-check when the window regains focus. Helps the default-app goal (W2-7).
3. **Keep the opened file before the first overwrite, and watch for outside changes.** A plain copy in local app data is enough for v1; glance's deduplicated store (FastCDC plus BLAKE3, `src-tauri/src/history.rs`) is a v2 pattern for full history. Pause autosave when the file changed on disk. Helps "revert to opened" and the save model (W2-3).
4. **Model autosave as a pure state machine with an injected clock.** Glance's rules are testable without a window: a quiet period, at most one write a minute, flush on blur, tab switch, and close, never mid-gesture, and no write when content equals the last save (`src/core/autosave.ts`, `tests/autosave.test.ts`). Helps W2-3.
5. **Store our own editable data beside standard annotations.** Glance writes each markup item as a standard subtype with an appearance stream, plus a private key that holds its editable geometry, and a document-level marker so open can skip extraction when there is no markup (`src/core/annotations.ts`). It also writes arrows as Line annotations by building the dictionary itself. That supports the option in SUMMARY decision 4 to write the Line dictionary ourselves instead of using Ink (D17). Use a second-class key prefix per the PDF spec. Helps task 5 (W2-4).

### Mistakes and risks to avoid

1. **Performance claims with no test behind them.** The 35 KB claim is off by a factor of 13, and pdf-lib loads at startup despite the README. Keep our rule: publish only numbers from the harness, with the commit ID.
2. **Docs that drift from code.** 4 ADR or comment claims disagree with the code (section 5). Update an ADR in the same change that alters its behavior, and link each ADR rule to a test.
3. **A file server or command with no trust boundary.** Any "read this path" or "write this path" endpoint must check its caller and its paths. Pin hashes for anything downloaded and executed. Do not let a caller add arguments or environment variables to a process it does not choose.
4. **Data-safety shortcuts.** Glance overwrites originals with no first-edit consent, continues when the safety copy fails, and renames without flushing. Our PRD asks once before the first overwrite. Our writes already call `sync_all` and `MoveFileExW` with `MOVEFILE_WRITE_THROUGH` (`src/pdf.rs`, `src/imaging.rs`). Keep both.
5. **Work and code that launch does not need.** Glance warms its model, opens a TCP listener, and schedules 2 opt-out network calls on every launch, and ships an unused HEVC decoder in the desktop build. Keep models on first use, telemetry opt-in, and run the license scan on the final package, not only on the lockfile.

### Glance better than our plan, and our plan stronger

| Topic | Better | Evidence |
| --- | --- | --- |
| Shell integration depth | Glance | Handler copy, `PerceivedType`, Send To, Explorer PDF thumbnailer, CI registry checks |
| Release engineering | Glance | `.github/workflows/*.yml`; we have no CI |
| Shipped breadth | Glance | 6 of 10 PRD tasks done and 4 partial, plus many v2 features, on the Store today |
| Background removal packaging | Glance | Pure-Rust `tract`, model embedded, works offline at first launch; we need an ONNX Runtime DLL and a model download |
| Speed method | Ours | Process-relaunch p95 harness and a 59-run release gate (`docs/benchmarks.md`); glance has no launch measurement |
| Rendering | Ours | Tiles, a byte-budget cache, and a newest-first cancelable queue (plan-v1 W2-1); glance uses whole-page canvases capped at 16 MP and an unbounded thumbnail cache |
| PDF saves | Ours | PDFium incremental save from a fresh handle (SUMMARY P3); glance rewrites the whole file on every markup save |
| Write durability | Ours | `sync_all` plus write-through move; glance renames with no flush |
| Resize size estimate | Ours (planned) | W1-C size estimate from an encode; glance shows uncompressed bytes |
| Privacy | Ours | No network on launch, opt-in telemetry; glance runs 2 opt-out calls 5 s after every launch |
| High contrast and UIA | Ours | Contrast theme and AccessKit; glance has no app forced-colors styles |
| Background removal quality | Ours | Snap model measured at 0.31 s for 12 MP with IoU 0.90 against BiRefNet (D16); glance masks at 320 × 320 with no quality measurement |
| First-overwrite consent | Ours (planned) | PRD asks once; glance overwrites silently and relies on version history |

## Confidence

| Finding | Confidence | Reason |
| --- | --- | --- |
| Test results | High | Ran on this PC; logs in the session scratchpad |
| Startup bundle 461 KB gzipped, pdf-lib loaded at startup | High | Built the same commit with the locked Vite version; the entry chunk imports the pdf-lib chunk |
| PRD coverage table | Medium | Read from code and README; no feature was exercised in a running app |
| `glance://` exposure to remote webviews | Medium for the code path, low for exploitability | Handler and Tauri registration read in source; whether WebView2 lets a remote page fetch `http://glance.localhost` is untested |
| Thumbnail loss when becoming default | Medium | Glance's installer comments and CI checks describe it; I did not reproduce it on Windows |
| libheif and libde265 in the desktop build | High | Chunk emitted by the desktop build; `de265` present in the bundle |
| DUTS-TR training-data risk | Medium | Web sources say research use with no explicit license; needs a lawyer like D16 |
| No launch measurement anywhere in glance | High | Searched README, docs, site, scripts, and tests |
| UI comparison | Medium | Glance screenshots are marketing renders; ours are wave 1 screenshots and may be out of date |

## Sources

- Glance repository: <https://github.com/RedtRocks/glance> (commit `78e091c`). Paths cited inline.
- Tauri 2.11.6 source, `src/manager/webview.rs`: <https://docs.rs/crate/tauri/2.11.6/source/src/manager/webview.rs> (read from the local cargo registry).
- U²-Net repository and license: <https://github.com/xuebinqin/U-2-Net>
- U²-Net paper: <https://arxiv.org/pdf/2005.09007>
- DUTS dataset terms: <https://saliencydetection.net/duts/>
- DUTS mirror under CC BY-NC 4.0: <https://huggingface.co/datasets/nobg/DUTS>
- libheif-js package: <https://www.npmjs.com/package/libheif-js>
- Umami (glance's install counter): <https://umami.is>
- Our context: `docs/PRD.md`, `docs/plan-v1.md`, `docs/research/SUMMARY.md`, `docs/benchmarks.md`, `CLAUDE.md` decisions D15 to D17.
