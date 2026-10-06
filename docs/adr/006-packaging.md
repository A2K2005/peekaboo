# ADR 006: Portable development package

Create an unsigned x64 ZIP containing the native executable, pinned PDFium DLL, hashes, and complete dependency notices. Offer a separate per-user Open with registration script. Do not change default apps automatically. The core package has been generated and is about 4.6 MB.

Signed MSIX, Store, and winget remain release work. No signing identity or reference-device QA is available in this task. The portable build lets the user test the app now without presenting it as a public release. ARM64 has not been built or tested.

Evidence: [Windows file associations](https://learn.microsoft.com/windows/win32/shell/fa-file-types), [MSIX signing](https://learn.microsoft.com/windows/msix/package/signing-package-overview), `../../tools/package.ps1`.
