# Preview for Windows

A native Rust desktop app for reading and editing PDFs and images. Files stay on the PC. There is no account, browser view, telemetry, or network request on launch.

This is a local development build. It is not a signed Store release or a claim that every original PRD acceptance criterion passes.

## Run

Open `dist/Preview/preview-for-windows.exe` after packaging, or `target/release/preview-for-windows.exe` after building. Keep `pdfium.dll` beside the executable. Open files with Ctrl+O, drag them into the window, or pass paths on the command line. The package includes an optional `register-file-associations.ps1` script for Open with; it does not change default apps.

## Available workflows

- Read PDFs and images; switch tabs; navigate pages and images; zoom and pan.
- Rotate, crop, move, insert blank pages, delete, extract, combine, and save PDF pages to a new file. Cropping is not redaction.
- Add PDF and image ink, highlights, underlines, strikeouts, notes, shapes, arrows, and text.
- Draw a signature, save it on this PC, and place it on another document.
- Fill supported AcroForm text fields and checkboxes through the field editor.
- Crop, rotate, flip, and resize images; export PNG, JPEG, WebP, TIFF, BMP, and image-to-PDF.
- Copy PDF page text or recognize image text offline using Windows OCR.
- Remove an image background with the optional local AI pack and save a transparent PNG.
- Print through the Windows print dialog, run a slideshow, and batch export image copies.
- Undo edits or revert to the file as opened. Originals are preserved; saving creates a new copy.

## Shortcuts

| Action | Shortcut |
| --- | --- |
| Open | Ctrl+O |
| Save a copy | Ctrl+Shift+S |
| Undo | Ctrl+Z |
| Rotate | Ctrl+R |
| Find PDF text | Ctrl+F |
| Copy text / image OCR | Ctrl+C |
| Fit / zoom | Ctrl+0 / Ctrl++ / Ctrl+- |
| Previous / next | Left / Right or Page Up / Page Down |
| Cancel gesture, batch, print, or slideshow | Escape |

## Important behavior

Save Copy never overwrites an existing file. Use a new filename. JPEG exports composite transparency onto white. WebP exports are lossless. Image exports rebuild from the original and the edit recipe; they do not save the smaller display preview. Background removal and image-to-PDF currently accept a maximum 64 MiB source raster, about 16 megapixels; larger sources need an explicit resize first.

Password-protected PDFs can be opened with a password kept in memory. Protected exports are blocked until encryption preservation is verified. Permission restrictions on text extraction and printing are enforced. XFA, JavaScript-dependent forms, digital-signature preservation, and full form-field interoperability are not complete. PDF merge and extraction refuse form documents rather than silently discard fields.

HEIC opens only when a working Windows codec is installed. No HEVC decoder is bundled. OCR needs an installed Windows OCR language. The AI pack is separate from the core app and loads only when background removal is requested. A 12 MP local background-removal test completed in 19.1 seconds on this development laptop, so the original three-second target is not met.

## Build and test

Requirements: Windows, Rust MSVC toolchain, Visual Studio C++ Build Tools, and the Windows SDK. No .NET SDK is needed for the app.

```powershell
pwsh -NoProfile -File tools/fetch-pdfium.ps1
cargo build --locked --release
pwsh -NoProfile -File tools/make-fixtures.ps1
cargo test --locked --release -- --test-threads=1
pwsh -NoProfile -File tools/package.ps1
```

See `docs/verification.md` for measured evidence, `docs/architecture.md` for module boundaries, and `docs/acceptance.md` for remaining requirements. The supplied PRD is preserved at `docs/PRD.md`. Later user instructions take precedence over its original stack and staged stop.
