# Acceptance status

This checklist distinguishes implemented workflows from the original PRD's stronger acceptance requirements. The owner authorized building directly and changing the stack. The full PRD is not passed.

| Area | Implemented | Remaining verification or implementation |
| --- | --- | --- |
| Open/read | Native PDF/image view, navigation, tabs, zoom/pan | Reference-device p95; continuous 500-page scrolling; thumbnail sidebar; 60 fps and next-image target |
| Find/copy | PDF search and full-page extraction | Mouse text selection, reading-order corpus, 500-page search under one second |
| Sign | Local saved ink signature and placement | Two-click acceptance, pen/touch/webcam paths, cross-reader validation |
| Forms | Text fields and checkboxes through native dialogs | Inline field navigation, radio/list controls, real 200-form corpus, XFA |
| Markup | Nine tools on PDFs/images; PDFium save/reopen evidence | Edge/Acrobat round trip, text-selection highlighting, more appearance cases |
| Pages | Rotate/crop/move/insert blank/delete/extract/merge | Thumbnail drag organization; image insert and Explorer drag-out; form-preserving imports |
| Crop/resize | Ordered full-resolution image edits and safe copies | Resulting size before save; color-profile corpus; very large-image stress |
| Convert/batch | JPEG/PNG/WebP/TIFF/BMP/PDF; batch copies | Quality slider; HEIC encoder legal/codec decision; 100-file usability/perf validation |
| OCR | Offline extraction verified on local Windows 10 | Hover word selection, multilingual/photo corpus, unavailable-language UX |
| Background | Full 12 MP transparent output verified | Three-second target fails; edge quality/Photos comparison; model distribution provenance review |
| Windows integration | Native dialogs, tabs, print code, slideshow | UIA document provider, Narrator, high contrast, 200% text scaling, pen/touch, share, signed MSIX/Store/winget |
| Saves | Non-overwriting Save Copy, undo/revert recipes | Incremental autosave and crash-recovery journal; encrypted export; signature validity |

The user stopped desktop control with Escape. No further desktop interaction was performed in that turn. Engine tests cannot substitute for UI acceptance.

Sources: [preserved PRD](PRD.md), [Windows accessibility](https://learn.microsoft.com/windows/apps/design/accessibility/accessibility), [PDFium API](https://pdfium.googlesource.com/pdfium/+/refs/heads/main/public/).
