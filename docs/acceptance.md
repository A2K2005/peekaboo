# Acceptance status

This checklist distinguishes implemented workflows from the original PRD's stronger acceptance requirements. The owner authorized building directly and changing the stack. The full PRD is not passed.

| Area | Implemented | Remaining verification or implementation |
| --- | --- | --- |
| Open/read | Native PDF/image view, tabs, continuous/single/two-page modes, zoom/pan, virtualized thumbnail/contents/notes sidebar, bounded tile cache, and neighbor-image predecode | Reference-device cold-launch p95; 500-page 60 fps and blank-tile targets; live next-image timing |
| Find/copy | PDF/OCR text layers, pointer and keyboard selection, word/line selection, reading-order copy, highlights, Ctrl+F and F3 navigation, and accessibility text | Final rerun and independent review of the latest fixes; persistent inline find UI; live pointer/Narrator/rotated-page/multi-frame corpus |
| Sign | Local saved ink signature and placement | Two-click acceptance, pen/touch/webcam paths, cross-reader validation |
| Forms | Text fields and checkboxes through native dialogs | Inline field navigation, radio/list controls, real 200-form corpus, XFA |
| Markup | Nine tools on PDFs/images; PDFium save/reopen evidence | Edge/Acrobat round trip, text-selection highlighting, more appearance cases |
| Pages | Rotate/crop/move/insert blank/delete/extract/merge | Thumbnail drag organization; image insert and Explorer drag-out; form-preserving imports |
| Crop/resize | Ordered full-resolution image edits and safe copies | Resulting size before save; color-profile corpus; very large-image stress |
| Convert/batch | JPEG/PNG/WebP/TIFF/BMP/PDF; batch copies | Quality slider; HEIC encoder legal/codec decision; 100-file usability/perf validation |
| OCR | Offline hover-triggered extraction, selectable text overlay, search/copy, and accessibility text | Final rerun after multi-frame selection fix; multilingual/photo corpus; unavailable-language UX |
| Background | Full 12 MP transparent output verified | Three-second target fails; edge quality/Photos comparison; model distribution provenance review |
| Windows integration | Native dialogs, tabs, print/share code, slideshow, single-instance handoff, recent files, Explorer registration, UIA control tree, dark/light/high-contrast rendering, and Mica with solid fallback | Narrator task completion, pen/touch, live Windows 11 composition, signed MSIX/Store/winget, and real Explorer install/uninstall |
| Saves | First-edit overwrite/save-copy choice with remembered policy, debounced autosave, immutable opened snapshots, incremental PDF updates, external-change conflicts, Revert to opened, persistent state, and close/logoff protection | Live interaction QA, dedicated retry action, crash-abandoned snapshot cleanup/recovery, encrypted export, and signature validity |

No GUI was launched during this update. Headless and integration tests cannot substitute for live UI acceptance.

Sources: [preserved PRD](PRD.md), [Windows accessibility](https://learn.microsoft.com/windows/apps/design/accessibility/accessibility), [PDFium API](https://pdfium.googlesource.com/pdfium/+/refs/heads/main/public/).
