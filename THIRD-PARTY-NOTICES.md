# Third-party components

The package includes exact license texts under `licenses/`. This list identifies the selected components; it is not a legal opinion or patent clearance.

| Component | Use | License and source |
| --- | --- | --- |
| Rust standard library | Native executable | MIT OR Apache-2.0 plus notices in the toolchain's `COPYRIGHT-library.html`; https://github.com/rust-lang/rust |
| windows-rs 0.62.2 and windows-numerics 0.3.1 | OS API bindings | MIT OR Apache-2.0; https://github.com/microsoft/windows-rs |
| PDFium 157.0.8086.0 | PDF parsing, rendering, forms, editing | PDFium license and every bundled dependency notice are retained; https://pdfium.googlesource.com/pdfium/+/refs/heads/main/LICENSE |
| bblanchon PDFium binary packaging | Pinned non-V8 x64 DLL | Packaging repository MIT; underlying PDFium/dependency terms still apply; https://github.com/bblanchon/pdfium-binaries |
| image-webp 0.2.4 | Lossless WebP export and fallback decode | MIT OR Apache-2.0; https://github.com/image-rs/image-webp |
| ort / ort-sys 2.0.0-rc.13 | Optional ONNX Runtime binding | MIT OR Apache-2.0; https://github.com/pykeio/ort |
| ONNX Runtime 1.24.4 | Optional local model execution | MIT and bundled third-party notices; https://github.com/microsoft/onnxruntime |
| BiRefNet lite ONNX | Optional background model, local evaluation | Official model card and code declare MIT. The supplied legal research flags training-data provenance for further distribution review; https://huggingface.co/ZhengPeng7/BiRefNet_lite and https://github.com/ZhengPeng7/BiRefNet/blob/main/LICENSE |
| WIC, Direct2D, Windows OCR, printer APIs | Installed OS services | Windows components; not redistributed by this project |

Cargo.lock and the package license folders include the exact transitive Rust versions. Their metadata was checked for GPL/AGPL dependencies. No GPL/AGPL reference implementation was copied. No HEVC, libheif, libde265, x265, MuPDF, or qpdf binary is in the core package.

The AI pack is optional and is not part of the default portable archive. Its current local development runtime came from the earlier supplied research installation. The model SHA-256 is `5600024376f572a557870a5eb0afb1e5961636bef4e1e22132025467d0f03333`. DirectML is not used by the app's verified CPU path. A public AI-pack release needs the unresolved provenance/distribution review and exact redistributable notices. No public release has been made.
