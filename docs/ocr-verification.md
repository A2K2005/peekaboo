# Offline OCR verification

Implemented `src/ocr.rs`: `recognize(path, edits)` decodes the edited source through WIC, limits dimensions to Windows OCR's maximum and the 64 MiB frame ceiling, composites transparency onto white, and passes a premultiplied BGRA `SoftwareBitmap` to Windows OCR. It selects an installed user-profile language. Missing support and empty recognition return explicit errors. There is no download or network code in this path.

Local evidence on 2026-10-06: the unpackaged Windows 10 x64 test executable recognized `Preview for Windows Offline text recognition 12345` from the generated 1200 by 260 PNG in **50.0861 ms**. This is one clean synthetic-text extraction, not an accuracy benchmark, cold-start distribution, all-language claim, network-isolation test, or proof of selectable hover text.

Generate the fixture and run the opt-in test:

```powershell
pwsh -NoProfile -File tools/make-fixtures.ps1
& C:/Users/Armaan/.cargo/bin/cargo.exe test --offline --release --test ocr_smoke -- --ignored --nocapture --test-threads=1
```

At the initial run, unrelated in-progress shell/PDF changes prevented the complete Cargo invocation from succeeding. The OCR test executable itself built and ran directly with the same test arguments. Re-run the full command once those changes compile. An independent agent must review this implementation; the author does not approve it.

Remaining checks: installed-language absence, handwritten/rotated/multilingual photos, very large images, transparent text fixtures, cancellation and interactive selection. OCR blocks its worker during recognition, not the window thread. It has no cancellation API in the current contract.

Sources: [OcrEngine](https://learn.microsoft.com/en-us/uwp/api/windows.media.ocr.ocrengine), [SoftwareBitmap buffer copy](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.imaging.softwarebitmap.createcopyfrombuffer), and the installed `windows` 0.62.2 generated bindings for `Media/Ocr`, `Graphics/Imaging`, and `Storage/Streams` verified against the compiling test.
