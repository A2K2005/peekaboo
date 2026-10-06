# ADR 003: WIC and WebP

Use WIC for the source-resolution edit/encode pipeline and Direct2D for markup. Use image-webp for lossless WebP export and fallback decode. Display-size buffers are separate from source-resolution output. Existing image tests demonstrate pixel, alpha, and dimension preservation.

WIC-only export cannot satisfy WebP on all target systems. Bundling ImageMagick adds an unnecessary second broad image stack. HEIC remains dependent on installed OS codecs until the specific HEVC licensing issue is resolved; it does not block other formats.

Evidence: [WIC](https://learn.microsoft.com/windows/win32/wic/-wic-about-windows-imaging-codec), [image-webp](https://github.com/image-rs/image-webp), `../../tests/image_workflow.rs`.
