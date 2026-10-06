# Product and UX teardown: Preview, Photos, Edge, PDFgear

- Date: 2026-10-06
- Author: Research agent: product and UX
- Status: draft

## Summary

- Preview covers all 10 v1 tasks in one window [1]. PDFgear covers 8, for PDFs only, and cannot remove backgrounds [2] [3]. Photos covers 5 image tasks and Edge covers 5 PDF tasks (derived from Part A).
- The PRD's XFA fallback ("Open in Edge") fails: Edge does not support XFA forms [4]. The PRD also says Photos has no markup on images, but Photos has pens, a highlighter, and arrows [5].
- On Windows 10, Mica falls back to a solid color, window corners stay square, and Segoe UI Variable and Segoe Fluent Icons are absent. We may not bundle either font [6] [7] [8] [9] [10].
- Microsoft conventions shape 4 choices. Ctrl+E is the Windows edit key, not the PRD's Ctrl+Shift+A [11]. Ctrl+9 selects the last tab, so Preview's fit key cannot move there [12] [13]. F5 and Ctrl+R mean "refresh" but are free in our app [11]. Text scaling reaches 225%, not the PRD's 200% [14].
- Explorer hides a multi-select verb above 15 files (Document model) or 100 files (legacy Player model). "Convert 100 selected files" needs a COM verb with the Player model [15].

## Findings

### How to read Part A

- "Clicks" counts mouse clicks. A drag counts as 1 drag, not a click. Typing is not counted.
- A count with "(derived from [n])" is our count of the documented steps. Nobody measured it.
- "Not found" means no primary page states it. "Not supported" means a primary page says the app cannot do it.
- Edge context: a policy page describes a new Edge PDF reader for Edge 111 and later with "no loss of functionality" [16]. Reports that it became the default in 2025 come only from a Message Center mirror. (unverified) The Edge rows below describe the documented reader.
- Photos context: many Photos features reach Windows Insiders first. The July 2026 redesign is in the Insider Experimental channel only [17]. General release dates on Windows 10 22H2 are not found.

### Part A. The 10 v1 tasks in 4 apps

#### A1. Open and read a PDF or image

| App | Entry point | Clicks or keys | Shortcuts | Defaults | Edge cases |
| --- | --- | --- | --- | --- | --- |
| Preview | Double-click in Finder; File > Open; File > Open Recent; Open With [18] | 1 double-click (derived from [18]) | Control-Tab next tab; Option-Down next page; Page Down next image; Option-Command-0 actual size; Option-Command-9 fit; Control-Command-F full screen [13] | First-open layout for PDFs is a setting: continuous, single, or two pages. Images can open as a group in one window or one window each [19] | Password: enter it in the Encryption inspector [20]. Huge images, HEIC: not found |
| Photos | Open from the gallery [21]; Explorer double-click when Photos is the default (unverified) | 1 click from the gallery [21] | F shows the filmstrip [22] | Zoom slider 10% to 800% [23]. Large images fit the window [21]. Filmstrip off [22] | Supported list: JPEG, PNG, GIF, BMP, TIFF. PDF is not listed [24]. HEIF and HEVC files may need a codec [25]. SVG viewing is Insider only [17] |
| Edge | Opens local, online, and embedded PDFs [4] | 1 double-click when Edge is the default (unverified) | Ctrl+\ fit page or width; Ctrl+0 reset zoom; F11 full screen; F7 caret browsing [26] [4] | Pinnable toolbar with zoom, rotate, fit, page jump, search. Table of contents pane. Single or two-page view [4] | Thumbnail pane: not found. Password PDFs: not found. Protected (MIP) PDFs open [4] |
| PDFgear | Double-click, Open File button, or Open with [27] | 1 double-click (derived from [27]) | Ctrl+0 fit page, Ctrl+1 actual size (unverified, third party [28]) | Single, double, continuous, auto-scroll, slide show. Background modes Default, Day, Night, Eye Protection, Yellow [29] | Opens PDFs only. Images enter through "Image to PDF" [2]. HEIC: not supported; it points to its online tool or Photos [30]. Password: a prompt asks for it [31] |

#### A2. Find and copy text

| App | Entry point | Clicks or keys | Shortcuts | Defaults | Edge cases |
| --- | --- | --- | --- | --- | --- |
| Preview | Search field in the toolbar [32]. Copy: Tools > Text Selection, drag, Edit > Copy [33] | Find: 1 click and type. Copy: 2 clicks, 1 drag, 2 clicks, or Command-C (derived from [32] [33] [13]) | Command-F, Command-G, Shift-Command-G (macOS standard [34]) | Results sort by Search Rank or Page Order. "Any Match" for phrase words [32] | Option-drag selects a column [33]. Password: unlock in the Encryption inspector before copy [20]. Scanned PDFs: not found |
| Photos | Scan Text button in the viewer [35] | 1 click, 1 drag (derived from [35]) | Arrow keys, Shift+Arrow, Home, End, Ctrl+A select detected text (Insider) [17] | Over 160 languages [35] | Images only. Find-in-image: not found |
| Edge | Ctrl+F or the search icon [26] [36] | 0 clicks with Ctrl+F (derived from [26]) | Ctrl+F; Ctrl+G or F3 next; Ctrl+Shift+G previous [26]. F7 caret mode, Shift+arrows select [4] | Not found | Scanned PDFs: OCR not found on a primary page |
| PDFgear | View tab > Find, or Ctrl+F [37]. Copy: drag, right-click > Copy [38] | Find: 0 clicks with Ctrl+F. Copy: 1 drag, 2 clicks (derived from [38]) | Ctrl+F [37] | Options "Case sensitive" and "Match all words". All matches yellow, current match red [37] | Scanned PDF: a prompt offers "Perform OCR" with page range and language [39] |

#### A3. Sign a document

| App | Entry point | Clicks or keys | Shortcuts | Defaults | Edge cases |
| --- | --- | --- | --- | --- | --- |
| Preview | Show Markup Toolbar > Sign > Trackpad, Camera, or iPhone [40] | Place a saved signature: 3 clicks and 1 drag (derived from [40]) | Not found | Signatures sync to other Macs through iCloud Drive. Delete with the X next to a signature [40] | Force Touch trackpads draw darker with pressure [40] |
| Photos | Not found | — | — | — | Photos does not open PDFs [24] |
| Edge | Draw on the toolbar, then pick color and width; ink is documented for signing [4] [41] | 2 clicks, then draw the signature by hand each time (derived from [41]) | Not found | Not found | "Add digital signatures" is listed as future work [42]. Edge views and validates certificate signatures [4] |
| PDFgear | Protect > Signature > Create Signature: Draw, Type, or Image [43] | Place a saved signature: about 4 clicks (derived from [43]) | Not found | Signatures are saved for later use [43] | Image signatures can drop a white background [44]. Certificate signing exists [43] [45] |

#### A4. Fill a form

| App | Entry point | Clicks or keys | Shortcuts | Defaults | Edge cases |
| --- | --- | --- | --- | --- | --- |
| Preview | Click a field and type. For forms without fields: Form Filling toolbar > Text Box [40] | 1 click per field (derived from [40]) | Tab between fields: not found | AutoFill from Contacts, where the PDF allows it [40] | XFA: not found |
| Photos | Not supported (no PDF) [24] | — | — | — | — |
| Edge | Click fields; "Add text" for non-form PDFs [4] [36] | 1 click per field; "Add text" 2 clicks (derived from [36]) | Not found | Basic form filling [4] | XFA "isn't supported". JavaScript forms aren't supported [4]. An IE-mode XFA policy needs Adobe's ActiveX plug-in [46] |
| PDFgear | Form tab: Add Text, Add Image, Insert Signature [47] | 1 click per field (derived from [48]) | Ctrl+H highlights form fields (unverified, third party [28]) | Form field highlighting since 2.1.8 [45] | XFA: not found. Tab-key navigation: not found |

#### A5. Mark up

| App | Entry point | Clicks or keys | Shortcuts | Defaults | Edge cases |
| --- | --- | --- | --- | --- | --- |
| Preview | Select text, then the Highlight button or its arrow menu. Markup toolbar: Sketch, Draw, Shapes, Text, Note, Sign [49] [50] | Highlight: 1 drag, 1 click. Change color: 2 clicks (derived from [49]) | Not found on Apple's pages | Highlight mode stays on until clicked again. Notes change color by Control-click [49] [51]. Default colors: not found | Images: annotations cannot be edited after save [52]. PDF text cannot be edited [50] |
| Photos | Edit image (Ctrl+E) > Markup: 2 pens, 1 highlighter, line, single arrow, double arrow [5] | 3 clicks before the first stroke (derived from [5]) | Ctrl+E [5] | Eraser and "Clear all markup" [5]. Default color: not found | Images only |
| Edge | Select text, right-click, Highlight, color. Add Comment for notes. Draw for ink [4] | Highlight: 1 drag, 3 clicks (derived from [4]) | Not found | 4 highlight colors in the 2020 reader [41]. Defaults: not found | Pen back button erases [41]. No shapes, arrows, or underline: not found |
| PDFgear | Highlight, Underline, Strikethrough, Area Highlight; Note, Line, Rectangle, Oval, Ink, Stamp [53] [54] | Highlight: 1 click, 1 drag (derived from [53]) | Alt+H, Alt+U and others (unverified, third party [28]) | About 50 preset highlight colors [55]. Default: not found | No arrow in the shape list [56]. Undo with Ctrl+Z [57] |

#### A6. Organize pages

| App | Entry point | Clicks or keys | Shortcuts | Defaults | Edge cases |
| --- | --- | --- | --- | --- | --- |
| Preview | View > Thumbnails or Contact Sheet; drag; Delete key; Edit > Insert > Page from File or Blank Page [58] | Reorder: 2 clicks to show thumbnails, then 1 drag (derived from [58]) | Delete key [58] | Changes save automatically [59] | Command-click selects separate pages [59]. Drag a thumbnail to the desktop to make a new PDF [58]. Drag a PDF from Finder into the sidebar to merge [59] |
| Photos | Not supported (no PDF) [24] | — | — | — | — |
| Edge | Rotate only, on the toolbar [4] | 0 clicks with keys [26] | Ctrl+] clockwise, Ctrl+[ counterclockwise [26] | — | Combine and other edits need a paid Acrobat subscription [60] |
| PDFgear | Page tab: Rotate, Delete Pages, Insert Pages, Extract Pages. Tools tab > Merge PDF [61] to [62] | Reorder: 2 clicks and 1 drag. Delete: about 5 clicks (derived from [61] [63]) | Not found on pdfgear.com | Merge result opens in a new window [62] | Reorder needs a "Yes" confirmation and "cannot be undone" [61] |

#### A7. Crop and resize images

| App | Entry point | Clicks or keys | Shortcuts | Defaults | Edge cases |
| --- | --- | --- | --- | --- | --- |
| Preview | Markup toolbar > Rectangular Selection > drag > Crop. Markup toolbar > Adjust Size [64] | Crop: 3 clicks, 1 drag. Resize: 2 clicks, type, 1 click (derived from [64]) | Not found | Adjust Size: Fit into, width and height, percent, "Scale proportionally", "Resample image"; new file size shows [64] | Several images: select in the sidebar, then Adjust Size [64]. Undo later through Revert To [64] |
| Photos | Edit image > Crop: presets, rotate, flip, free rotation [5]. Resize tool exists [22] | Crop: 2 clicks, then 1 drag (derived from [5]) | Ctrl+E; Space compares with the original [5] | Resize by pixels or percent, quality setting, file-size preview (unverified; secondary [65]) | Batch resize: not found |
| Edge | Not supported (PDF viewer only) [4] | — | — | — | — |
| PDFgear | Page tab > Crop Page, for PDF pages only [66] | 2 clicks, 1 drag, 1 click (derived from [66]) | Not found | Exact margins with "Adjust Selected Area" [66] | Image files: not found. Older guide: one page at a time [66] |

#### A8. Convert and batch

| App | Entry point | Clicks or keys | Shortcuts | Defaults | Edge cases |
| --- | --- | --- | --- | --- | --- |
| Preview | File > Export > Format [67] | 5 clicks (derived from [67]) | Not found | Formats: HEIC, JPEG, JPEG-2000, OpenEXR, PDF, PNG, TIFF. JPEG has a Quality slider. Option-click shows more formats [67] | WebP is not in the list [67]. Batch: select images in the sidebar, then export [67] |
| Photos | Format conversion: not found on a primary page. After edits: Save as copy, or overwrite [5] | — | Not found | HEIC stays HEIC when saving edits (Insider) [17] | Resize saves JPG, PNG, TIF, or BMP (unverified; secondary [65]). Batch: not found |
| Edge | Save or save a copy of the PDF [26] [36] | 1 click (derived from [36]) | Ctrl+S [26] | PDF only | Conversion needs a paid Acrobat subscription [60] |
| PDFgear | Tools tab > Convert > PDF to PNG or JPG; Image to PDF [68] [2] | About 4 clicks (derived from [68]) | Not found | Page range, Quality, Color Mode [69]. "Output in One File" for image to PDF [2] | Several PDFs at once [70]. Image inputs: BMP, ICO, JPEG, JPG, PNG [2]. WebP or TIFF output: not found |

#### A9. Copy text from an image

| App | Entry point | Clicks or keys | Shortcuts | Defaults | Edge cases |
| --- | --- | --- | --- | --- | --- |
| Preview | Hover over text, drag, Control-click > Copy Text [71] | 1 drag, 2 clicks (derived from [71]) | Not found | Look Up and Translate on the same menu [71] | "Not available in all regions or languages" [71] |
| Photos | Scan Text button [35] | 1 click, 1 drag (derived from [35]) | Keyboard selection (Insider) [17] | Over 160 languages [35] | Windows 11 and Windows 10 Release Preview [35]. Snipping Tool text actions are Windows 11 only [72] |
| Edge | Not found for scanned PDFs | — | — | — | — |
| PDFgear | Home tab > OCR > drag > Done > copy or save as TXT [73] | 4 clicks, 1 drag (derived from [73]) | Not found | Language choice in the dialog [73] | Internet is listed only for chat, help, and updates, so OCR is likely offline (derived from [74]) |

#### A10. Remove a background

| App | Entry point | Clicks or keys | Shortcuts | Defaults | Edge cases |
| --- | --- | --- | --- | --- | --- |
| Preview | Remove Background button [75] | 1 click, plus 1 click on the PNG prompt (derived from [75]) | Shift-Command-K [13] | Prompt: Cancel makes a duplicate; Convert changes the original to PNG [75] | PDFs: export the page as an image first [75] |
| Photos | Edit image > Background > Blur, Remove, or Replace [5] | 3 clicks, plus save (derived from [5]) | Ctrl+E [5] | Remove gives a transparent background. A brush adds or removes areas. Runs on the device [5] | Windows 10 and Arm64 since version 2024.11020.21001.0; no hardware requirement stated [76]. Save or copy to the clipboard [22] |
| Edge | Not supported (PDF viewer only) [4] | — | — | — | — |
| PDFgear | Not supported. PDFgear points users to remove.bg [3] | — | — | — | Only signature and stamp images can drop a white background [44] |

### Part B. "Also in v1" items

| Item | Preview | Photos | Edge | PDFgear |
| --- | --- | --- | --- | --- |
| Tabs | Window tabs; Control-Tab and Control-Shift-Tab [13]; "Prefer tabs" setting [77] | Not found | Browser tabs; Ctrl+T, Ctrl+W [26] | Not found. Merge opens a new window [62] |
| Slideshow | View > Slideshow, with on-screen controls [78] | Slideshow with transitions and music [79] | Not found | Home tab > Slide Show; arrows to move [80] |
| Print | File > Print; Command-P; print selected thumbnails, notes, scale to fit [81] [13] | Print in the top bar (Insider) [17] | Ctrl+P [26] | Annotations on or off, page range, duplex, scaling [82] |
| Share | Share button [83] | Right-click > Share [24] | Not found | Share button, email only [84] |
| Themes | Window background color setting [19] | Not found | Follows the OS high-contrast setting [4] | System, Light, Dark [29] [85] |
| Shortcut coverage | 21 general shortcuts, more in menus [13] | No official list; Ctrl+E, Space, F [5] [22] | PDF keys Ctrl+\, Ctrl+[, Ctrl+] [26] | Shortcut keys since 2.1.10 [45]; full list only on a third-party site [28] |
| Undo | Edit > Undo Crop [86]; Command-Z [34] | Reset cancels all edits [5]. Ctrl+Z: not found | Not found | Ctrl+Z and an Undo button [57]. Page reorder cannot be undone [61] |
| Revert | File > Revert To > Last Opened, Last Saved, Previous Save, Browse All Versions [87] | Reset; Space to compare with the original [5] | Not found | Not found |
| Save model | Autosave [88] | Save as copy, or overwrite from the dropdown [5] | Ctrl+S; save a copy from the toolbar [26] [36] | "Save as" [57]. Autosave and close prompt: not found |

#### How Preview autosave and Revert work

1. Preview saves changes automatically while you work [88].
2. It keeps snapshots called versions. It saves a version at least every hour, and when you open, save, duplicate, lock, rename, or revert [88].
3. File > Revert To lists Last Opened, Last Saved, and Previous Save, plus Browse All Versions [87].
4. Option-click "Restore a Copy" in the version browser makes a new file instead of replacing the current one [87].
5. macOS saves unsaved changes on close by default. The setting "Ask to keep changes when closing documents" turns on a prompt [89].
6. Apple's own advice before combining PDFs: "choose File > Duplicate" to keep the originals [59].

Windows has no per-file version store that our app can call. So "Revert to opened" needs our own copy of the original. (unverified)

#### The first-edit prompt in other apps

- Preview does not ask. It overwrites the original and relies on versions [88] [87].
- Preview asks once in one case: Remove Background on a non-PNG offers Cancel (make a duplicate) or Convert (change the original) [75].
- Photos offers "Save as copy" first, and overwrite second, through a dropdown [5].
- Edge saves with Ctrl+S or saves a copy from the toolbar [26] [36]. A close prompt: not found.
- No app of the four has a "Remember my choice" option. (not found)

### Part C. Fluent 2 and Windows 11 design guidance for a WinUI 3 app

The stack is open again (CLAUDE.md decision D3). This part applies to WinUI 3. Any other stack must reproduce these values by hand to look native.

#### C1. Mica, Mica Alt, and Windows 10

| Topic | Guidance | Source |
| --- | --- | --- |
| Mica | Background for long-lived windows. Apply it as the base layer and keep it visible in the title bar. | [90] |
| Mica Alt | Stronger tint, "especially when creating an app with a tabbed title bar". Needs Windows App SDK 1.1 and Windows 11 build 22000. | [90] |
| API | `Window.SystemBackdrop = new MicaBackdrop()`. Set `Kind="BaseAlt"` for Mica Alt. Apply the backdrop once per window, never to a control. | [90] [91] |
| Windows 10 | Mica is Windows 11 only and falls back to a solid theme color on Windows 10. Desktop Acrylic works on Windows 10 build 17763 and later. | [6] |
| Other fallbacks | Solid color also when transparency effects are off, over Remote Desktop, on weak GPUs, and in high contrast. | [6] |
| Fallback brushes | `SolidBackgroundFillColorBase` (Mica) and `SolidBackgroundFillColorBaseAlt` (Mica Alt). | [90] |
| Doc conflict | The Mica page says Battery Saver forces the fallback. The materials page says Mica is not affected. | [90] [6] |

Result: on Windows 10 the app gets a flat background with no extra code. Every surface must read well as a solid color.

#### C2. Title bar and tabs in the title bar

| Topic | Guidance | Source |
| --- | --- | --- |
| Height | The standard title bar is 32 px. Use 48 px when it holds a search box or a person picture. | [92] |
| Behavior | All empty space drags the window. Right-click shows the system menu. Double-click maximizes. | [92] |
| Tabs | Tabs may use the title bar. Caption buttons stay on the right. | [92] |
| TabView in title bar | Always include a `TabStripFooter` and mark it as the drag region. Microsoft's sample sets `MinWidth = 188`. | [12] |
| Clickable items in the title bar | Use `InputNonClientPointerSource.SetRegionRects` with `Passthrough`. Recalculate on resize and scale by `RasterizationScale`. | [93] |
| Tall caption buttons | `PreferredHeightOption = Tall` with a 48 px title bar. It throws if `ExtendsContentIntoTitleBar` is false. | [93] |
| TitleBar control | Added in Windows App SDK 1.7. Caption buttons are not part of it. | [93] [94] |
| Windows 10 | Customization works on Windows 10 1809 and later. Title bar color properties are ignored on Windows 10. | [93] [95] |
| Full screen | The system hides the title bar. | [93] |

#### C3. Typography

Windows 11 type ramp, in effective pixels (epx) [96]:

| Style | Size / line height | Weight |
| --- | --- | --- |
| Caption | 12/16 | Regular |
| Body | 14/20 | Regular |
| Body Strong | 14/20 | Semibold |
| Body Large | 18/24 | Regular |
| Subtitle | 20/28 | Semibold |
| Title | 28/36 | Semibold |
| Title Large | 40/52 | Semibold |
| Display | 68/92 | Semibold |

- Minimum sizes are 14 px Semibold and 12 px Regular. Use sentence case. Use Semibold, not Bold [96].
- Fluent 2 uses the same ramp with Segoe UI Variable for Windows [97].
- The Windows 10 font list has Segoe UI but not Segoe UI Variable [8] [98].
- Segoe UI Variable "is not available for licensing or use outside of Microsoft products" [9]. We cannot bundle it.
- The title bar page says to use "Segoe UI Variable (if available) or Segoe UI" [92]. The font WinUI 3 picks on Windows 10 22H2 is not stated. (unverified)

#### C4. Spacing and layout

- Sizes, margins, and positions use multiples of 4 epx [99]. Fluent 2 also uses a 4 px base unit [100].
- Breakpoints: small up to 640 epx, medium 641 to 1007, large 1008 and up [99].
- 8 epx between buttons and between a control and its header. 12 epx between a control and its label, and between content areas. 16 epx between a surface edge and text [101].
- WinUI has no general-purpose compact mode. Start with default control sizes [102].
- Standard control heights and page margins: not found on a current Microsoft page. (unverified)

#### C5. Corner radius

- 8 px for windows, flyouts, and dialogs. 4 px for in-page controls, bars, and tooltips. 0 px where straight edges meet, and when a window is snapped or maximized [103].
- Resources: `ControlCornerRadius` is 4 px and `OverlayCornerRadius` is 8 px [103].
- Windows 11 rounds WinUI window corners [104]. The DWM corner attribute starts at Windows 11 build 22000 [7]. So Windows 10 windows stay square (derived from [7]).

#### C6. Iconography

- Segoe Fluent Icons replaced Segoe MDL2 Assets in Windows 11. MDL2 is still available [10].
- Segoe Fluent Icons is not on Windows 10. Its download is for design and development [10]. Microsoft does not allow redistribution of Windows fonts [9].
- `SymbolThemeFontFamily` falls back to Segoe MDL2 Assets on "Windows 10, version 20H2 or earlier" [105]. Behavior on 22H2 is not stated. (unverified)
- Icons are monoline with a 1 epx stroke. A 16 epx font size gives a 16 x 16 icon [106].
- Our toolbar glyphs exist in both fonts. `Pen` (F67B) is Fluent-only [10] [107].

| Tool | Glyph | Code point |
| --- | --- | --- |
| Sidebar | OpenPane | E8A0 |
| Zoom | Zoom, ZoomIn, ZoomOut | E71E, E8A3, E71F |
| Markup | Edit | E70F |
| Rotate | Rotate | E7AD |
| Share | Share | E72D |
| Search | Search | E721 |
| Highlight | Highlight | E7E6 |
| Sign | SignatureCapture | EF3F |
| Crop | Crop | E7A8 |
| Print | Print | E749 |
| Undo, Redo | Undo, Redo | E7A7, E7A6 |
| Slideshow | Slideshow | E786 |
| Note | Comment | E90A |
| Draw | InkingTool | E76D |

#### C7. Motion

- Duration resources: 83 ms (faster), 167 ms (fast), 250 ms (normal) [108].
- Enter with `cubic-bezier(0, 0, 0, 1)`. Exit with `cubic-bezier(1, 0, 1, 1)` [108].
- Direct entrance takes 167, 250, or 333 ms. Fades are linear, 83 ms. "Avoid custom animations where possible" [109].
- The motion page lists direct exit as `(0, 0, 0, 1)`, which contradicts [108] [109].
- Apps should respond to `UISettings.AnimationsEnabled` [110].

#### C8. Accessibility

| Area | Guidance | Source |
| --- | --- | --- |
| Narrator | Give icon-only controls an `AutomationProperties.Name`. The accessible name is the most important property. | [102] [111] |
| Accelerators | Buttons add the accelerator to their tooltip. Narrator announces `AcceleratorKey`. A focused TextBox wins over app accelerators. | [11] |
| Access keys | Alt shows keytips. A one-character key also works as Alt+key. Narrator reads the access key on focus. Scope with `IsAccessKeyScope`. | [112] |
| Contrast themes | Use `ThemeDictionaries` with a `HighContrast` key and `SystemColor` resources. Use 2 px borders on flyouts and dialogs. | [113] |
| Text scaling | Users can scale text from 100% to 225%. WinUI apps support it by default. Set `IsTextScaleFactorEnabled=false` on font icons. Prefer clipping to ellipses. | [14] |
| Focus visuals | Primary border 2 px, secondary 1 px, margin 1 px. | [114] |
| Targets | 7.5 mm, about 40 x 40 px at 135 PPI. | [115] |
| Keyboard | Tab and Shift+Tab; arrows inside groups; Esc closes transient UI; F6 and Shift+F6 cycle panes. No first focus on a destructive action. | [116] |
| Single-key shortcuts | A letter-only shortcut must be able to turn off, be remapped, or work only when its control has focus (WCAG 2.1, level A). | [117] |

## Interaction spec

These are our decisions. Facts that drive a decision carry a source. A decision with no source is marked "Decision".

### Layout rules (from the PRD)

- Toolbar, 6 buttons: Sidebar, Zoom, Markup, Rotate, Share, Search. A "More" (...) button holds the rest.
- The markup bar is hidden. It opens on the Markup button, Ctrl+E, or Ctrl+Shift+A.
- No home screen. An empty window shows recent files and "New from clipboard".
- Every command has a right-click entry, an access key, and, where a free key exists, a Ctrl accelerator. Microsoft calls access keys keyboard shortcuts [112].
- At 320 epx only 4 primary commands fit in a command bar [118]. Overflow order, last to first: Share, Rotate. Decision.

Access keys for the toolbar (Alt, then the letter, or Alt+letter directly [112]):

| Button | Access key |
| --- | --- |
| Sidebar | S |
| Zoom | Z |
| Markup | M |
| Rotate | R |
| Share | H |
| Search | F |
| More | O |

Access keys inside the markup bar (scope M; for example Alt, M, H): Select S, Highlight H, Underline U, Strikethrough K, Note N, Text box T, Arrow A, Shapes P, Draw D, Sign G, Crop C, Resize Z, Remove background B, Color L. Decision.

### D1. Open and read (PRD task 1)

| Item | Spec |
| --- | --- |
| Entry points | Explorer double-click. Explorer "Open with" on a multi-select opens one window with one tab per file. Drag files onto the window or the tab strip. Ctrl+O. Ctrl+T opens a file in a new tab [12]. Recent files in an empty window. |
| Shortest path | 1 double-click in Explorer, 0 clicks in the app. |
| Shortcuts | Ctrl+O, Ctrl+T, Ctrl+W, Ctrl+Tab, Ctrl+1 to Ctrl+9 [12]. Ctrl+= and Ctrl+- zoom; Ctrl+0 fit [11]. Ctrl+\ toggles fit page and fit width [26]. Page Up, Page Down, Home, End. Left and Right go to the previous and next image in the folder. Ctrl+G goes to a page [119]. F11 full screen [26]. Alt+Enter shows file info [120]. Ctrl+Shift+B toggles the sidebar. F6 cycles panes [11]. |
| Defaults | Images fit the window and never enlarge past 100%. PDFs open in continuous scroll, fit to width, at the last page viewed (Preview has this option [19]). Zoom steps always include 100%, so Ctrl+= lands on actual size. Decision. |
| Password PDF | An inline password box in the tab, not a modal dialog. Enter opens. A wrong password keeps the box and says "Wrong password". Decision. |
| Huge image | Decode at display size first, full size on zoom (PRD rule). After 1 s, show the slim progress bar (PRD rule). |
| HEIC without codec | Use the bundled decoder (PRD). If legal review blocks it, show an InfoBar with a link to the Windows extension. Microsoft says HEIF files "may need" a codec [25]. |
| Corrupt or unsupported file | An error page inside the tab with "Open with another app". Decision. |
| PRD criterion | Task 1: first page under 400 ms p95; 60 fps scroll on a 500-page PDF. |

### D2. Find and copy text (PRD task 2)

| Item | Spec |
| --- | --- |
| Entry points | Search toolbar button; Ctrl+F; right-click on selected text > "Find". |
| Shortest path | Ctrl+F, type, Enter: 0 clicks. |
| Shortcuts | Ctrl+F. F3 or Enter next; Shift+F3 or Shift+Enter previous [11]. Esc closes. Ctrl+C copies. Ctrl+A selects all text on the page. |
| Copy | The pointer over text is an I-beam. No tool switch is needed (Preview needs Tools > Text Selection [33]). Alt+drag selects a column (maps Preview's Option-drag [33]). |
| Defaults | Case-insensitive, partial words (PDFgear offers both options [37]). The box shows "3 of 27". With more than 1 match, the sidebar lists results by page. Decision. |
| Scanned PDF | The find box says "No text on these pages" with a "Recognize text" button. It runs the task 9 engine on demand (PDFgear offers the same [39]). |
| Copy-protected PDF | Honor the permission. Say "The author blocked copying." Decision. |
| PRD criterion | Task 2: Ctrl+F finds text in a 500-page PDF in under 1 s; copy keeps reading order. |

### D3. Sign a document (PRD task 3)

| Item | Spec |
| --- | --- |
| Entry points | Sign button in the markup bar (lists saved signatures and "Create signature"). Right-click on a page > "Sign here". Access key Alt, M, G. |
| Shortest path | Right-click at the spot > "Sign here" places the last-used signature: 2 clicks. With the markup bar open: Sign > click the page: 2 clicks. Preview needs 3 clicks and 1 drag (derived from [40]). |
| Create | A pad for mouse, pen, or touch, plus webcam capture (PRD). Pen pressure sets stroke width. Buttons "Clear" and "Save". |
| Defaults | Black ink. Stored only on this PC, never synced. Placed at 40 mm wide. Shift keeps the aspect ratio while resizing (PDFgear does the same [45]). Decision. |
| Certificate signature field | Explain: "This is a digital-signature field. This version adds a drawn signature only." Decision. |
| Already signed PDF | Warn that a new mark changes the file after signing. Edge can validate certificate signatures [4], so users can see the change. Decision. |
| Images | The signature merges into the pixels on save, as Preview does with image markup [52]. |
| PRD criterion | Task 3: drawn once with mouse, pen, or touch; saved locally; placed in 2 clicks. |

### D4. Fill a form (PRD task 4)

| Item | Spec |
| --- | --- |
| Entry points | Open the PDF. Fields get a light tint (PDFgear has field highlighting [45]). Click a field and type, as in Preview [40]. |
| Shortest path | 1 click, then type. |
| Shortcuts | Tab and Shift+Tab move between fields. Space toggles a checkbox. Arrows move in a radio group. Alt+Down opens a list. |
| Non-form PDF | Text box tool (Alt, M, T) or right-click > "Add text here": 2 clicks. Edge has the same "Add text" flow [36]. |
| Text box defaults | Black, Helvetica 11 pt, so Acrobat and Edge show it without an embedded font. Decision. |
| XFA form | InfoBar: "This form uses XFA. Edge can't fill it either. Open it in Adobe Acrobat Reader." Button "Open with…" shows the Windows app picker. Edge does not support XFA [4]. |
| JavaScript form | InfoBar: "Some fields calculate with scripts. Check totals before you send." Edge does not run JavaScript forms [4]. Decision. |
| PRD criterion | Task 4: AcroForm fields typeable and Tab-navigable; free text boxes on non-form PDFs. |

### D5. Mark up (PRD task 5)

| Item | Spec |
| --- | --- |
| Entry points | Markup toolbar button, Ctrl+E, or Ctrl+Shift+A. Right-click on selected text > Highlight, Underline, Strikethrough, Add note. Pen on screen draws ink with no tool picked (PRD). |
| Shortest path | Select text, Ctrl+Shift+H: 0 clicks. Select text, right-click > Highlight: 2 clicks. Edge needs 3 clicks after the selection (derived from [4]). |
| Markup bar order | Select, Highlight (split button: colors, underline, strikethrough), Note, Text box, Shapes (rectangle, oval, line, arrow), Draw, Sign, then Color and Line width. Images add Crop, Resize, Remove background. |
| Shortcuts | Ctrl+Shift+H highlight; Ctrl+U underline [11]. Every other tool has an access key in scope M. Delete removes the selected mark. |
| Defaults | Highlight yellow. Note yellow. Shapes and arrows red, 2 pt. Ink black, 2 pt, pressure on. Each tool remembers its last color. No app of the four documents its defaults (not found). Decision. |
| Author name | Off by default. Preview adds a name only when you set one [19]. Decision. |
| Notes list | Sidebar "Notes" tab, like Preview's Highlights and Notes [49]. |
| Images | Marks stay editable until the tab closes. On save they merge into the pixels, as in Preview [52]. Say so once in an InfoBar. Decision. |
| Scanned PDF | Text highlight needs a text layer. Offer "Recognize text", or use an area highlight. Decision. |
| PRD criterion | Task 5: 8 tools; saved as standard PDF annotations readable in Acrobat and Edge. |

### D6. Organize pages (PRD task 6)

| Item | Spec |
| --- | --- |
| Entry points | Sidebar thumbnails (Sidebar button or Ctrl+Shift+B). Right-click on a thumbnail: Rotate left, Rotate right, Delete, Insert blank page after, Insert from file, Export pages, Copy. Drag inside the sidebar to reorder. Drop a file from Explorer into the sidebar to insert it at the drop point. Drag thumbnails to Explorer to make a new PDF. Preview supports both drags [59] [58]. |
| Shortest path | Reorder: 1 drag, 0 clicks. PDFgear needs 2 clicks and 1 drag and has no undo [61]. |
| Multi-select | Click, Ctrl+click, Shift+click, Ctrl+A [11]. |
| Shortcuts | Delete removes pages with undo, no confirmation [11]. Ctrl+R and Ctrl+L rotate. Ctrl+Shift+N inserts a blank page, scoped to the sidebar [11]. Ctrl+X and Ctrl+V move pages by keyboard. Decision. |
| Defaults | A blank page copies the size of the page before it. A dropped image becomes a page sized to the image. Decision. |
| Encrypted PDF | Saving page edits must keep the encryption. The PRD says PDFium cannot encrypt on save, so route the save through qpdf. |
| Dropped protected PDF | Ask for its password before insert. Decision. |
| Last page | Delete is disabled when 1 page is left. Decision. |
| PRD criterion | Task 6: reorder, delete, rotate, insert blank or image page; drag out creates a PDF; drop in merges. |

### D7. Crop and resize images (PRD task 7)

| Item | Spec |
| --- | --- |
| Entry points | Drag a rectangle on the image with the normal pointer. A "Crop" button appears next to it. Right-click > Crop. Markup bar Crop and Resize. Right-click > Resize. |
| Shortest path | Crop: 1 drag, then Enter: 0 clicks. Resize: Ctrl+Shift+R, type, Enter: 0 clicks. Preview needs 3 clicks and 1 drag to crop (derived from [64]). |
| Shortcuts | Enter applies, Esc cancels. Shift keeps a square. Ctrl+K crops to the selection. Ctrl+Shift+R opens Resize. Decision. |
| Resize dialog | Width, height, unit (pixels or percent), aspect lock, resample. Shows the new size in pixels and the estimated file size, as Preview does [64]. |
| Defaults | Aspect lock on, pixels, resample on. JPEG saves at the source quality estimate, or 90 if unknown. Decision. |
| PDF pages | The same drag and Crop set the page crop box. Preview's crop hides content [86]. Say "Cropped areas are hidden, not deleted." Offer "Apply to all pages" (PDFgear's older guide crops 1 page at a time [66]). |
| Huge image | Preview at display size; apply at full size in the background with the slim progress bar (PRD rule). |
| Several images | Resize applies to every selected image. See D8. |
| PRD criterion | Task 7: crop by drag; resize by pixels or percent with aspect lock; shows file size before save. |

### D8. Convert and batch (PRD task 8)

| Item | Spec |
| --- | --- |
| Entry points | More > Export as (Ctrl+Shift+E). More > Save a copy (Ctrl+Shift+S). Explorer right-click on a multi-select: Convert, Resize, Combine into PDF (PRD). In the app: select images in the sidebar (Ctrl+A), then Export or Resize. |
| Shortest path | One file: Ctrl+Shift+E, pick a format (2 clicks), Enter. Preview needs 5 clicks (derived from [67]). 100 files from Explorer: right-click, Convert, Start: 3 clicks. |
| Export dialog | Format: JPEG, PNG, WebP, TIFF, HEIC, PDF. Quality slider for JPEG, WebP, HEIC. Estimated size. Folder defaults to the source folder. A name clash adds " (2)". |
| Defaults | JPEG 85, WebP 80, HEIC 80. PDF to images: 1 file per page, 150 dpi. Images to PDF: 1 PDF, sidebar order, page size equals image size. Decision; validate in the imaging research. |
| Explorer limit | Explorer hides a legacy verb above 15 items (Document model) or 100 items (Player model). A COM verb with the Player model has no limit [15]. Use that. |
| Errors | Batch never stops on one failure. It ends with "98 converted, 2 failed" and a details list. Decision. |
| Transparency to JPEG | Flatten on white. Decision. |
| HEIC export | Needs an HEVC encoder. If none is available, disable HEIC and say why. (unverified; see imaging research) |
| PRD criterion | Task 8: export to 6 formats with a quality slider; resize, rotate, or convert 100 files in one action. |

### D9. Copy text from an image (PRD task 9)

| Item | Spec |
| --- | --- |
| Entry points | Hover over text in an image: the pointer becomes an I-beam, as in Preview [71]. Drag to select. Right-click > Copy text or Copy all text. A small "Text" button appears when text is found, as Photos shows Scan Text [35]. |
| Shortest path | Drag, Ctrl+C: 0 clicks. Copy all: right-click > Copy all text: 2 clicks. |
| Shortcuts | Ctrl+C. Ctrl+Shift+C copies all text. With the image focused, Ctrl+A selects all text and Shift+arrows extend it (Photos added the same keys [17]). Decision. |
| When OCR runs | On a background thread after the image shows. The model loads on first use (PRD rule). |
| Scanned PDF pages | Same hover-and-select. Tasks 2 and 9 share one engine. Decision. |
| No text | No I-beam and no Text button. Decision. |
| Missing language | Say which Windows language pack adds it. Decision; confirm in the imaging research. |
| PRD criterion | Task 9: hover shows selectable text; works offline on all supported PCs. |

### D10. Remove a background (PRD task 10)

| Item | Spec |
| --- | --- |
| Entry points | Ctrl+Shift+K (maps Preview's Shift-Command-K [13]). Markup bar > Remove background. Right-click on the image > Remove background. |
| Shortest path | 1 key, or 2 clicks. Photos needs 3 clicks (derived from [5]). |
| Result | Shows in place on a checkerboard. Ctrl+Z restores, as in Preview [75]. Ctrl+C copies a PNG with transparency. |
| Saving a JPEG source | Ask once: "Save as a PNG copy" (default) or "Convert this file to PNG". Preview asks the same question [75]. The first-edit rule's "Remember my choice" applies. |
| PDFs | Disabled, with the tooltip "Export the page as an image first", as in Preview [75]. |
| No subject | Say "No clear subject found." Decision. |
| Refine | Photos has a brush to add or remove areas [5]. The PRD has no brush in v1. See Open questions. |
| PRD criterion | Task 10: one click; under 3 s on a non-AI laptop; copy or save as transparent PNG. |

### D11. Also in v1

| Item | Spec |
| --- | --- |
| Tabs | TabView in the title bar with a `TabStripFooter` drag region [12]. Mica Alt behind tabs on Windows 11 [90]. Ctrl+T, Ctrl+W, Ctrl+F4, Ctrl+Shift+T, Ctrl+Tab, Ctrl+1 to Ctrl+9 [12]. Tear-out to a new window needs Windows App SDK 1.6 [12]. |
| Slideshow | F5 from page 1, Shift+F5 from the current page, Esc ends, as in PowerPoint [121]. Space and arrows move. |
| Print | Ctrl+P opens the Windows print dialog through `PrintManagerInterop` [122]. Options: page range, include marks. |
| Share | Share button and right-click > Share open the Windows share sheet through `IDataTransferManagerInterop` [123]. Behavior on Windows 10: not found. |
| Themes | Follow the system light or dark theme. Support contrast themes with `ThemeDictionaries` [113]. |
| Undo | Ctrl+Z; Ctrl+Y and Ctrl+Shift+Z redo [11] [34]. Undo covers page edits too (PDFgear cannot undo a reorder [61]). |
| Revert to opened | The "Edited" label next to the file name in the title bar opens a flyout with "Revert to opened": 2 clicks. Before the first write, copy the original to the app's local data folder. Delete the copy when the tab closes. Decision. |
| File info | Alt+Enter [120]. |

### D12. First-edit prompt, autosave, and close

1. The first edit applies at once in memory. Nothing is written yet.
2. An InfoBar under the toolbar asks: "Save edits to this file?" Buttons: "Edit this file" and "Save as a copy". Checkbox: "Remember my choice". The InfoBar does not block work (PRD: edits are non-blocking).
3. "Edit this file": autosave 2 s after the last edit and when the tab closes. Revert to opened stays available. Decision.
4. "Save as a copy": the first save asks for a name next to the original ("name (edited).pdf"), then autosaves to the copy. Decision.
5. Ignored prompt, then close: a ContentDialog "Save edits to name.pdf?" Buttons: "Save" (primary), "Don't save" (secondary), and "Cancel" as the safe close button on the right. A "Save as a copy" link sits in the dialog text. No default button. Microsoft wants a safe close button on the right, specific verbs, and no default unless one is chosen [124].
6. "Remember my choice" is a setting, with a reset in Settings.

### Consolidated keyboard shortcuts

"Conflict" lists a clash with Windows, WinUI, Microsoft conventions, Edge, or Preview. "Precedent" is the source that supports the choice.

| Action | Shortcut | Precedent | Conflict or note |
| --- | --- | --- | --- |
| Open | Ctrl+O | Word [119] | None in [11] |
| Open in new tab | Ctrl+T | TabView guidance [12]; Edge [26] | None |
| New from clipboard | Ctrl+N | "Add a new item" [11] | None |
| Close tab | Ctrl+W, Ctrl+F4 | [12] [11] | Ctrl+F4 is built into TabView [12] |
| Reopen closed tab | Ctrl+Shift+T | [12] [26] | Keeps Ctrl+Shift+T off any markup tool |
| Next, previous tab | Ctrl+Tab, Ctrl+Shift+Tab | Built into TabView [12] | None |
| Tab 1 to 8, last tab | Ctrl+1 to Ctrl+8, Ctrl+9 | [12] [26] | Preview's fit key maps to Ctrl+9 [13]; we use Ctrl+0 instead. PDFgear uses Ctrl+1 to Ctrl+6 for zoom (unverified [28]) |
| Save | Ctrl+S | [11] | With autosave, Ctrl+S forces a save now |
| Save a copy | Ctrl+Shift+S | Preview Duplicate, Shift-Command-S [34] | Word uses F12 for Save As [119] |
| Export as | Ctrl+Shift+E | None | Explorer uses it to expand folders, inside Explorer only [120] |
| Print | Ctrl+P | [11] | None |
| Find | Ctrl+F | [11] | None |
| Find next, previous | F3, Shift+F3; Enter, Shift+Enter in the box | [11] | Edge also uses Ctrl+G for next [26] |
| Go to page | Ctrl+G | Word [119] | **Conflict:** Edge uses Ctrl+G for find next [26] |
| Copy | Ctrl+C | [11] | A focused text box handles it first [11] |
| Copy all text in image | Ctrl+Shift+C | None | None in [11] |
| Select all | Ctrl+A | [11] | None |
| Undo, redo | Ctrl+Z; Ctrl+Y, Ctrl+Shift+Z | [11] [34] | Inside a text box, undo edits the text [11] |
| Zoom in, out | Ctrl+=, Ctrl+-, Ctrl+wheel, pinch | [11] [26] | None |
| Fit to window | Ctrl+0 | "Zoom to default view" [11] | Word's Ctrl+0 resets to 100% [119] |
| Fit page or width | Ctrl+\ | Edge [26] | Key position varies by keyboard layout (unverified) |
| Full screen | F11 | Edge [26] | None |
| Slideshow | F5; Shift+F5 | PowerPoint [121] | **Conflict:** F5 means refresh [11] [120]. The app has nothing to refresh |
| Sidebar | Ctrl+Shift+B | None | Edge uses it for the favorites bar [26] |
| Next pane | F6, Shift+F6 | [11] [116] | None |
| Markup bar | Ctrl+E (shown); Ctrl+Shift+A (alias) | Ctrl+E: [11] [5]. Ctrl+Shift+A: PRD; Preview's key is not on Apple's pages (unverified) | **Conflict with PRD:** PRD names only Ctrl+Shift+A |
| Highlight selection | Ctrl+Shift+H | None | None found in [120] |
| Underline selection | Ctrl+U | [11] | None |
| Rotate right, left | Ctrl+R, Ctrl+L; aliases Ctrl+], Ctrl+[ | Edge [26]; Preview's Command-R and Command-L (unverified) | **Conflict:** Ctrl+R means refresh [11]. The app has nothing to refresh |
| Crop to selection | Ctrl+K | Preview Command-K (unverified) | None in [11] |
| Resize | Ctrl+Shift+R | None | None in [11] |
| Remove background | Ctrl+Shift+K | Preview Shift-Command-K [13] | None found in [120] |
| Delete page or mark | Delete | "Delete selected item (with undo)" [11] | None |
| Insert blank page | Ctrl+Shift+N, sidebar only | "Add a new secondary item" [11] | Explorer uses it for a new folder, inside Explorer only [120] |
| File info | Alt+Enter | "Display properties" [120] | None |
| Every other tool | Alt, then access keys | [112] | Scopes avoid clashes [112] |
| Avoid | Alt+F4, Alt+Space, Alt+Tab, F10, Shift+F10, Ctrl+Esc, Ctrl+Shift+Esc, Ctrl+Shift alone | Windows uses them [120] | Do not assign |
| Avoid | Ctrl+Alt+any key | Ctrl+Alt acts as AltGr on many layouts (unverified) | Do not assign |
| Localize | All letters | Spanish Windows uses Ctrl+N for bold [11] | English first (PRD) |

## Confidence

| Finding | Confidence | Reason |
| --- | --- | --- |
| Preview task steps | High | Apple's current guide (macOS 27 edition) [1] |
| Preview shortcuts beyond Apple's list | Low | Apple lists 21 general keys and points to the menus [13]. Markup, crop, and rotate keys are unverified |
| Edge feature set and gaps | Medium | The Learn page was updated in June 2026 [4], but the new reader's UI may differ (unverified) |
| Photos feature set | Medium | Support pages and Insider blogs [5] [35]. Many features reach Insiders first; general release on Windows 10 22H2 is not found |
| Photos resize details | Low | Secondary source only [65] |
| PDFgear feature set | Medium | Own help pages, but parts describe an older UI [47] [53]; shortcuts only on a third-party site [28] |
| Windows 10 visuals (Mica, corners, fonts, icons) | High | Microsoft Learn [6] [7] [8] [9] [10] |
| Explorer 15 and 100 item limits | High | Microsoft Learn [15] |
| XFA not supported in Edge | High | Microsoft Learn [4] |
| Our shortcut map | Medium | Checked against Microsoft's lists [11] [120] [12]; not tested with users |
| Click counts | Medium | Derived from documented steps, not measured |

## Conflicts with the PRD

1. **XFA fallback.** The PRD risk table says "Open in Edge" for XFA. Edge does not support XFA [4]. Its IE-mode workaround needs Adobe's ActiveX plug-in [46].
2. **Photos markup.** The PRD says Photos has "no markup on top of images". Photos has 2 pens, a highlighter, and arrow lines [5].
3. **Windows 10 look.** The PRD theme row names Mica, rounded corners, and Segoe UI Variable. None exists on Windows 10, and we may not bundle the font or Segoe Fluent Icons [6] [7] [8] [9] [10]. Windows 10 is a PRD target.
4. **Text scale target.** The PRD says 200%. Windows goes to 225% [14].
5. **Markup bar shortcut.** The PRD names Ctrl+Shift+A. Windows convention and Photos use Ctrl+E for editing [11] [5]. Apple's pages do not list Shift-Command-A. (unverified)
6. **"Convert 100 files" from Explorer.** A legacy verb disappears above 15 files (Document) or 100 files (Player) [15]. The design needs a COM verb with the Player model.
7. **Toolbar width.** 6 primary buttons do not fit at 320 epx; 4 do [118]. The PRD must accept overflow at narrow widths.
8. **"Every action has a keyboard shortcut."** Free Ctrl chords run out. The rule holds only if access keys count [112].
9. **"Revert to opened, one click away."** Preview relies on macOS versions [87] [88]. We must store our own copy. A destructive revert should also confirm, which makes it 2 clicks. Decision.
10. **Competitor baseline.** Photos removes backgrounds on Windows 10 and Arm64 with no stated hardware need [76], and reads text in over 160 languages [35]. The PRD's "on par with Photos" gate must include Windows 10 PCs and OCR languages.
11. **Preview inventory row.** The PRD lists WebP among Preview's convert formats. Apple's list has no WebP [67]. WebP export is our addition.

## What we should do

1. Replace "Open in Edge" for XFA with "Open with…" and name Adobe Acrobat Reader. Do the same for script-heavy forms.
2. Fix the PRD problem table: Photos does mark up images. Our edge is editable marks on PDFs and images in one app.
3. Design a Windows 10 skin: solid `SolidBackgroundFillColorBase` background, square corners, Segoe UI, and Segoe MDL2 Assets glyphs. Use only glyphs that exist in both icon fonts, or ship our own icon set as vector paths.
4. Change the accessibility target to 225% text scale and test every screen at that size.
5. Make Ctrl+E the shown markup shortcut and keep Ctrl+Shift+A as an alias. Owner decision.
6. Build the Explorer verbs as COM commands with `MultiSelectModel=Player`, not legacy command lines, so more than 100 files work [15].
7. Give every command an access key, and give Ctrl accelerators only to the keys in the table above.
8. Ship the first-edit InfoBar and the close dialog exactly as in D12. Store the original before the first write so "Revert to opened" works.
9. Add "Sign here" to the page right-click menu. It is the only 2-click path that does not need the markup bar open.
10. Reserve Ctrl+1 to Ctrl+9, Ctrl+T, Ctrl+Shift+T, and Ctrl+W for tabs. Do not reuse Preview's Ctrl+9 fit key.
11. Benchmark background removal and OCR against Photos on a Windows 10 PC, not only on Windows 11.
12. Re-test Edge's PDF reader on a current Windows 11 PC before the beta. The reader may have changed.

## Open questions

1. Ctrl+E or Ctrl+Shift+A as the primary markup shortcut?
2. Is "Revert to opened" allowed 2 clicks (open flyout, confirm)?
3. How long do we keep the original copy for revert: until the tab closes, or several days?
4. Do we add a refine brush to background removal in v1, as Photos has [5]?
5. Which defaults (highlight color, export quality, signature width) should the beta test? No competitor documents its defaults.
6. Can we draw our own icon set, or do we accept MDL2 glyphs on Windows 10?
7. What does the current Edge PDF reader do with password PDFs and unsaved marks on close? A hands-on test on Windows 11 can answer this.
8. What are the Photos resize defaults on the current general release? Only a secondary source documents them [65].
9. Unchecked: Preview menu shortcuts for the markup toolbar (Shift-Command-A), crop (Command-K), and rotate (Command-L, Command-R). Apple's pages do not list them [13].
10. Unchecked: whether Ctrl+Alt shortcuts clash with AltGr, and whether Ctrl+\ and Ctrl+[ work on non-US layouts. Test on German and French layouts.
11. Unchecked: which text font and icon font WinUI 3 uses on Windows 10 22H2, and whether the Windows share sheet works there [105] [123].
12. Unchecked: whether an HEVC encoder is available for HEIC export without the Store extension. The imaging research owns this.
13. Unchecked: whether Edge's newer PDF reader changed the toolbar, password handling, or the close prompt [16]. Needs a hands-on test.
14. Unchecked: Photos keyboard shortcuts beyond Ctrl+E, Space, and F. No current official list was found.

## Sources

1. Apple: Preview User Guide (macOS 27 edition), table of contents. https://support.apple.com/guide/preview/welcome/mac
2. PDFgear: Create PDF from image (Windows user guide). https://www.pdfgear.com/windows-user-guide/create-pdf-from-image.htm
3. PDFgear: PDF to transparent PNG. https://www.pdfgear.com/how-to/pdf-to-transparent-png.htm
4. Microsoft Learn: PDF reader in Microsoft Edge. https://learn.microsoft.com/en-us/deployedge/microsoft-edge-pdf
5. Microsoft Support: Edit photos and videos in Windows (Photos). https://support.microsoft.com/en-us/windows/edit-photos-and-videos-in-windows-a3a6e711-1b70-250a-93fa-ef99048a2c86
6. Microsoft Learn: Materials overview. https://learn.microsoft.com/en-us/windows/apps/develop/ui/materials
7. Microsoft Learn: DWMWINDOWATTRIBUTE enumeration. https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute
8. Microsoft Learn: Windows 10 font list. https://learn.microsoft.com/en-us/typography/fonts/windows_10_font_list
9. Microsoft Learn: Font redistribution FAQ. https://learn.microsoft.com/en-us/typography/fonts/font-faq
10. Microsoft Learn: Segoe Fluent Icons font. https://learn.microsoft.com/en-us/windows/apps/design/style/segoe-fluent-icons-font
11. Microsoft Learn: Keyboard accelerators. https://learn.microsoft.com/en-us/windows/apps/develop/input/keyboard-accelerators
12. Microsoft Learn: Tab View. https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/tab-view
13. Apple: Keyboard shortcuts in Preview on Mac. https://support.apple.com/guide/preview/keyboard-shortcuts-cpprvw0003/mac
14. Microsoft Learn: Text scaling. https://learn.microsoft.com/en-us/windows/apps/develop/input/text-scaling
15. Microsoft Learn: How to employ the verb selection model (Explorer multi-select limits). https://learn.microsoft.com/en-us/windows/win32/shell/how-to-employ-the-verb-selection-model
16. Microsoft Learn: Edge policy NewPDFReaderEnabled. https://learn.microsoft.com/en-us/deployedge/microsoft-edge-policies/newpdfreaderenabled
17. Microsoft Learn: Photos release notes (Windows Insider). https://learn.microsoft.com/en-us/windows-insider/release-notes/apps/photos
18. Apple: Open PDFs and images in Preview. https://support.apple.com/guide/preview/open-pdfs-and-images-prvw81f73d4e/mac
19. Apple: Change Preview settings. https://support.apple.com/guide/preview/change-preview-settings-prvw5518ecc0/mac
20. Apple: If you can't select or copy text in a PDF in Preview. https://support.apple.com/guide/preview/if-you-cant-select-or-copy-text-in-a-pdf-prvw1499/mac
21. Windows Insider blog: Photos update with super resolution (Oct 2024). https://blogs.windows.com/windows-insider/2024/10/22/microsoft-photos-update-with-super-resolution-begins-rolling-out-to-windows-insiders/
22. Windows Insider blog: Photos gets background remove and replace (Nov 2023). https://blogs.windows.com/windows-insider/2023/11/17/windows-photos-gets-background-remove-and-replace-along-with-other-improvements/
23. Windows Insider blog: Viewer and import enhancements for Photos (June 2024). https://blogs.windows.com/windows-insider/2024/06/20/viewer-and-import-enhancements-for-microsoft-photos-on-windows-11/
24. Microsoft Support: Manage photos and videos with the Microsoft Photos app. https://support.microsoft.com/en-us/windows/apps/photos/manage-photos-and-videos-with-microsoft-photos-app
25. Microsoft Support: Upload HEIF and HEVC photos and videos to OneDrive. https://support.microsoft.com/en-us/office/upload-heif-and-hevc-photos-and-videos-to-onedrive-96d137f5-369b-4d99-9db8-523c736da425
26. Microsoft Support: Keyboard shortcuts in Microsoft Edge. https://support.microsoft.com/en-us/microsoft-edge/keyboard-shortcuts-in-microsoft-edge-50d3edab-30d9-c7e4-21ce-37fe2713cfad
27. PDFgear: Open a PDF (Windows user guide). https://www.pdfgear.com/windows-user-guide/open-a-pdf.htm
28. DefKey: PDFgear Windows shortcuts (third party, unverified). https://defkey.com/pdfgear-windows-shortcuts
29. PDFgear: Navigate PDF (Windows user guide). https://www.pdfgear.com/windows-user-guide/navigate-pdf.htm
30. PDFgear: How to convert HEIC to PDF. https://www.pdfgear.com/pdf-converter/how-to-convert-heic-to-pdf-for-free.htm
31. PDFgear: Open a locked PDF (Windows user guide). https://www.pdfgear.com/windows-user-guide/open-a-locked-pdf.htm
32. Apple: Find text in PDFs in Preview. https://support.apple.com/guide/preview/find-text-in-pdfs-prvw2014/mac
33. Apple: Select and copy text in a PDF in Preview. https://support.apple.com/guide/preview/select-and-copy-text-in-a-pdf-prvw1020/mac
34. Apple: Mac keyboard shortcuts. https://support.apple.com/en-us/102650
35. Windows Insider blog: March 2025 Microsoft Photos update (Scan Text). https://blogs.windows.com/windows-insider/2025/03/24/march-2025-microsoft-photos-update-now-rolling-out-to-windows-insiders/
36. Microsoft: Edge PDF reader feature page. https://explore.microsoft.com/en-us/edge/features/pdf-reader
37. PDFgear: Find text in PDF (Windows user guide). https://www.pdfgear.com/windows-user-guide/find-text-in-pdf.htm
38. PDFgear: How to copy text from PDF. https://www.pdfgear.com/pdf-editor-reader/how-to-copy-text-from-pdf.htm
39. PDFgear: How to search in a PDF. https://www.pdfgear.com/pdf-editor-reader/how-to-search-in-a-pdf.htm
40. Apple: Fill out and sign PDF forms in Preview. https://support.apple.com/guide/preview/fill-out-and-sign-pdf-forms-prvw35725/mac
41. Microsoft Tech Community: Microsoft Edge extends tools for the PDF reader (2020). https://techcommunity.microsoft.com/discussions/edgeinsiderannouncements/microsoft-edge-extends-tools-for-the-pdf-reader/1303243
42. Microsoft Tech Community: Roadmap for PDF reader in Microsoft Edge. https://techcommunity.microsoft.com/discussions/edgeinsiderannouncements/roadmap-for-pdf-reader-in-microsoft-edge/2175170
43. PDFgear: Sign PDF on Windows. https://www.pdfgear.com/sign-pdf/sign-pdf-on-windows.htm
44. PDFgear: Add signature image to PDF. https://www.pdfgear.com/sign-pdf/add-signature-image-to-pdf.htm
45. PDFgear: What's new (release notes). https://www.pdfgear.com/whats-new/
46. Microsoft Learn: Edge policy ViewXFAPdfInIEModeAllowedOrigins. https://learn.microsoft.com/en-us/deployedge/microsoft-edge-policies/viewxfapdfiniemodeallowedorigins
47. PDFgear: Fill out PDF form (Windows user guide). https://www.pdfgear.com/windows-user-guide/fill-out-pdf-form.htm
48. PDFgear: PDF form product page. https://www.pdfgear.com/pdf-form/
49. Apple: Highlight, underline, and strike out text in Preview. https://support.apple.com/guide/preview/highlight-underline-and-strike-out-text-prvw757d4d5b/mac
50. Apple: Annotate a PDF in Preview. https://support.apple.com/guide/preview/annotate-a-pdf-prvw11580/mac
51. Apple: Add notes and speech bubbles to a PDF in Preview. https://support.apple.com/guide/preview/add-notes-and-speech-bubbles-to-a-pdf-prvw7450efd7/mac
52. Apple: Annotate an image in Preview. https://support.apple.com/guide/preview/annotate-an-image-prvw1501/mac
53. PDFgear: Markup PDF (Windows user guide). https://www.pdfgear.com/windows-user-guide/markup-pdf.htm
54. PDFgear: How to add comments to a PDF. https://www.pdfgear.com/how-to/how-to-add-comments-to-a-pdf.htm
55. PDFgear: Change highlight color in PDF. https://www.pdfgear.com/pdf-editor-reader/change-highlight-color-in-pdf.htm
56. PDFgear: Add shape to PDF. https://www.pdfgear.com/pdf-editor-reader/add-shape-to-pdf.htm
57. PDFgear: How to remove highlight in PDF. https://www.pdfgear.com/pdf-editor-reader/how-to-remove-highlight-in-pdf.htm
58. Apple: Add, delete, or move PDF pages in Preview. https://support.apple.com/guide/preview/add-delete-or-move-pdf-pages-prvw11793/mac
59. Apple: Combine PDFs in Preview. https://support.apple.com/guide/preview/combine-pdfs-prvw43696/mac
60. Microsoft Edge blog: Adobe Acrobat and Microsoft Edge PDF (2023). https://blogs.windows.com/msedgedev/2023/02/08/adobe-acrobat-microsoft-edge-pdf/
61. PDFgear: Reorder PDF pages (Windows user guide). https://www.pdfgear.com/windows-user-guide/reorder-pdf-pages.htm
62. PDFgear: Merge and split PDFs (Windows user guide). https://www.pdfgear.com/windows-user-guide/merge-and-split-pdfs.htm
63. PDFgear: Delete page from PDF (Windows user guide). https://www.pdfgear.com/windows-user-guide/delete-page-from-pdf.htm
64. Apple: Crop, resize, or rotate an image in Preview. https://support.apple.com/guide/preview/crop-resize-or-rotate-an-image-prvw2015/mac
65. Office Watch: Image resize options in Windows Photos (secondary source). https://office-watch.com/2024/great-image-resize-options-now-in-windows/
66. PDFgear: Crop pages (Windows user guide). https://www.pdfgear.com/windows-user-guide/crop-pages.htm
67. Apple: Convert image file types in Preview. https://support.apple.com/guide/preview/convert-image-file-types-prvw1012/mac
68. PDFgear: Convert PDF to image (Windows user guide). https://www.pdfgear.com/windows-user-guide/convert-pdf-to-image.htm
69. PDFgear: Crop a PDF to JPG (quality and color mode settings). https://www.pdfgear.com/pdf-converter/how-to-crop-a-pdf-to-jpg.htm
70. PDFgear: Batch convert PDF (Windows user guide). https://www.pdfgear.com/windows-user-guide/batch-convert-pdf.htm
71. Apple: Interact with text in a photo in Preview. https://support.apple.com/guide/preview/interact-with-text-in-a-photo-prvw625a5b2c/mac
72. Microsoft Support: Use Snipping Tool to capture screenshots. https://support.microsoft.com/en-us/windows/use-snipping-tool-to-capture-screenshots-00246869-1843-655f-f220-97299b865f6b
73. PDFgear: OCR PDF (Windows user guide). https://www.pdfgear.com/windows-user-guide/ocr-pdf.htm
74. PDFgear: Download and install PDFgear on Windows. https://www.pdfgear.com/windows-user-guide/download-install-pdfgear-on-windows.htm
75. Apple: Remove a background or extract an image in Preview. https://support.apple.com/guide/preview/remove-a-background-or-extract-an-image-prvw15636/mac
76. Windows Insider blog: Photos AI editing on Arm64 and Windows 10 (Feb 2024). https://blogs.windows.com/windows-insider/2024/02/22/windows-photos-gets-generative-erase-and-recent-ai-editing-features-now-available-on-arm64-devices-and-windows-10/
77. Apple: Use tabs in windows on Mac. https://support.apple.com/guide/mac-help/use-tabs-in-windows-mchla4695cce/mac
78. Apple: Display a PDF as a slideshow in Preview. https://support.apple.com/guide/preview/display-a-pdf-as-a-slideshow-prvw3b268798/mac
79. Windows Insider blog: Photos app update for Windows 11 (May 2023, slideshow). https://blogs.windows.com/windows-insider/2023/05/03/photos-app-for-windows-11-update-brings-improvements-for-windows-insiders/
80. PDFgear: PDF full screen. https://www.pdfgear.com/pdf-editor-reader/pdf-full-screen.htm
81. Apple: Print PDFs and images in Preview. https://support.apple.com/guide/preview/print-pdfs-and-images-prvw15175/mac
82. PDFgear: PDF print settings (Windows user guide). https://www.pdfgear.com/windows-user-guide/pdf-print-settings.htm
83. Apple: Share PDFs or images in Preview. https://support.apple.com/guide/preview/share-pdfs-or-images-prvwf246acd7/mac
84. PDFgear: Share PDF (Windows user guide). https://www.pdfgear.com/windows-user-guide/share-pdf.htm
85. PDFgear: How to read PDF in dark mode. https://www.pdfgear.com/pdf-editor-reader/how-to-read-pdf-in-dark-mode.htm
86. Apple: Crop or rotate a PDF in Preview. https://support.apple.com/guide/preview/crop-or-rotate-a-pdf-prvw11567/mac
87. Apple: Revert changes to PDFs and images in Preview. https://support.apple.com/guide/preview/revert-changes-to-pdfs-and-images-prvwf4697c5a/mac
88. Apple: Save files in Preview. https://support.apple.com/guide/preview/save-files-prvw0b1e8d9e/mac
89. Apple: Change Desktop & Dock settings on Mac. https://support.apple.com/guide/mac-help/change-desktop-dock-settings-mchlp1119/mac
90. Microsoft Learn: Mica material. https://learn.microsoft.com/en-us/windows/apps/design/style/mica
91. Microsoft Learn: Apply system backdrops (Mica, Acrylic). https://learn.microsoft.com/en-us/windows/apps/develop/ui/system-backdrops
92. Microsoft Learn: Title bar design. https://learn.microsoft.com/en-us/windows/apps/design/basics/titlebar-design
93. Microsoft Learn: Title bar customization. https://learn.microsoft.com/en-us/windows/apps/develop/title-bar
94. Microsoft Learn: TitleBar control. https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/title-bar
95. Microsoft Learn: Windows App SDK 1.2 release notes. https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/release-notes/windows-app-sdk-1-2
96. Microsoft Learn: Typography in Windows. https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/typography
97. Fluent 2: Typography. https://fluent2.microsoft.design/typography
98. Microsoft Learn: Windows 11 font list. https://learn.microsoft.com/en-us/typography/fonts/windows_11_font_list
99. Microsoft Learn: Screen sizes and breakpoints. https://learn.microsoft.com/en-us/windows/apps/design/layout/screen-sizes-and-breakpoints-for-responsive-design
100. Fluent 2: Layout. https://fluent2.microsoft.design/layout
101. Microsoft Learn: Content basics (spacing). https://learn.microsoft.com/en-us/windows/apps/design/basics/content-basics
102. Microsoft Learn: Design line-of-business apps. https://learn.microsoft.com/en-us/windows/apps/get-started/line-of-business/design-for-lob
103. Microsoft Learn: Geometry (corner radius). https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/geometry
104. Microsoft Learn: Apply rounded corners in desktop apps. https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/ui/apply-rounded-corners
105. Microsoft Learn: Icons in WinUI (SymbolThemeFontFamily). https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/icons
106. Microsoft Learn: Iconography. https://learn.microsoft.com/en-us/windows/apps/design/iconography/
107. Microsoft Learn: Segoe MDL2 Assets icons. https://learn.microsoft.com/en-us/windows/apps/design/style/segoe-ui-symbol-font
108. Microsoft Learn: Timing and easing. https://learn.microsoft.com/en-us/windows/apps/design/motion/timing-and-easing
109. Microsoft Learn: Motion in Windows. https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/motion
110. Microsoft Learn: Tailor effects and experiences (AnimationsEnabled). https://learn.microsoft.com/en-us/windows/apps/develop/composition/composition-tailoring
111. Microsoft Learn: Accessibility overview. https://learn.microsoft.com/en-us/windows/apps/design/accessibility/accessibility-overview
112. Microsoft Learn: Access keys. https://learn.microsoft.com/en-us/windows/apps/develop/input/access-keys
113. Microsoft Learn: Contrast themes. https://learn.microsoft.com/en-us/windows/apps/design/accessibility/high-contrast-themes
114. Microsoft Learn: Guidelines for visual feedback (focus visuals). https://learn.microsoft.com/en-us/windows/apps/develop/input/guidelines-for-visualfeedback
115. Microsoft Learn: Guidelines for touch targets. https://learn.microsoft.com/en-us/windows/apps/develop/input/guidelines-for-targeting
116. Microsoft Learn: Keyboard interactions. https://learn.microsoft.com/en-us/windows/apps/develop/input/keyboard-interactions
117. W3C: Understanding WCAG 2.1 SC 2.1.4 Character Key Shortcuts. https://www.w3.org/WAI/WCAG21/Understanding/character-key-shortcuts.html
118. Microsoft Learn: Command bar. https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/command-bar
119. Microsoft Support: Keyboard shortcuts in Word. https://support.microsoft.com/en-us/office/keyboard-shortcuts-in-word-95ef89dd-7142-4b50-afb2-f762f663ceb2
120. Microsoft Support: Keyboard shortcuts in Windows. https://support.microsoft.com/en-us/windows/keyboard-shortcuts-in-windows-dcc61a57-8ff0-cffe-9796-cb9706c75eec
121. Microsoft Support: Keyboard shortcuts to deliver PowerPoint presentations. https://support.microsoft.com/en-us/office/use-keyboard-shortcuts-to-deliver-powerpoint-presentations-1524ffce-bd2a-45f4-9a7f-f18b992b93a0
122. Microsoft Learn: Print from your app. https://learn.microsoft.com/en-us/windows/apps/develop/devices-sensors/print-from-your-app
123. Microsoft Learn: Integrate with the Windows share sheet (desktop apps). https://learn.microsoft.com/en-us/windows/apps/develop/windows-integration/integrate-sharesheet-send
124. Microsoft Learn: Dialog controls. https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/dialogs-and-flyouts/dialogs
