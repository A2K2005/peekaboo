# Third-party components

Distribution packages must include exact license texts under `licenses/`. This list identifies the selected components; it is not a legal opinion or patent clearance.

| Component | Use | License and source |
| --- | --- | --- |
| Rust standard library | Native executable | MIT OR Apache-2.0 plus notices in the toolchain's `COPYRIGHT-library.html`; https://github.com/rust-lang/rust |
| windows-rs 0.62.2 and windows-numerics 0.3.1 | OS API bindings | MIT OR Apache-2.0; https://github.com/microsoft/windows-rs |
| PDFium 157.0.8086.0 | PDF parsing, rendering, forms, editing | PDFium license and every bundled dependency notice are retained; https://pdfium.googlesource.com/pdfium/+/refs/heads/main/LICENSE |
| bblanchon PDFium binary packaging | Pinned non-V8 x64 DLL | Packaging repository MIT; underlying PDFium/dependency terms still apply; https://github.com/bblanchon/pdfium-binaries |
| image-webp 0.2.4 | Lossless WebP export and fallback decode | MIT OR Apache-2.0; https://github.com/image-rs/image-webp |
| ort / ort-sys 2.0.0-rc.13 | Optional ONNX Runtime binding | MIT OR Apache-2.0; https://github.com/pykeio/ort |
| ONNX Runtime 1.24.4 | Optional local model execution | MIT and bundled third-party notices; https://github.com/microsoft/onnxruntime |
| libwebp-sys 0.14.4 | Statically linked lossy WebP encoder bindings | The published Cargo manifest declares MIT, but the crate contains no binding license text or copyright notice. Release packaging is blocked until the exact upstream notice is obtained or the binding is replaced. The package audit retains the published manifest and VCS provenance without inventing a notice; https://github.com/NoXF/libwebp-sys |
| libwebp (vendored by libwebp-sys) | Lossy WebP encoding | BSD-3-Clause and additional patent grant. Retain COPYING and PATENTS from the vendored source; https://github.com/webmproject/libwebp |
| AccessKit 0.25.1 and accesskit_windows 0.35.1 | Accessibility tree and Windows UI Automation | MIT OR Apache-2.0; https://github.com/AccessKit/accesskit |
| withoutbg Snap matting and refiner 0.1.0 | Optional local background-removal models | Apache-2.0 according to the publisher's model card; https://huggingface.co/withoutbg/snap |
| Depth Anything V2 Small, Snap slim ONNX conversion | Depth stage of the optional AI pack | Apache-2.0 for Small and the published Snap conversion. This does not apply to the larger non-commercial variants; https://github.com/DepthAnything/Depth-Anything-V2#license and https://huggingface.co/withoutbg/snap |
| WIC, Direct2D, Windows OCR, printer APIs | Installed OS services | Windows components; not redistributed by this project |

Cargo.lock and the package license folders include the exact transitive Rust versions. Their metadata was checked for GPL/AGPL dependencies. No GPL/AGPL reference implementation was copied. No HEVC, libheif, libde265, x265, MuPDF, or qpdf binary is in the core package.

The AI pack is optional. Its three model filenames and SHA-256 values are pinned in `tools/fetch-model.ps1` and checked again by `src/background.rs`. The app uses CPU inference; DirectML is not enabled on this path. The publisher's declared licenses are recorded above, not an independent clearance of training-data rights. A distributable AI pack must include the model license texts and ONNX Runtime's exact third-party notices. No public release has been made.
