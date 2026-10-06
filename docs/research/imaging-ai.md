# Imaging and on-device AI: research findings

Date: 2026-10-06
Author: Research agent: imaging and AI
Status: draft. The owner's usage budget ran out partway through. Timing benchmarks for decode, OCR, and background removal did not run. See "Open questions".

All measured numbers come from the dev PC: Windows 10 Home 22H2, build 19045.6456, i7-11800H, 32 GB, RTX 3050 Ti Laptop and Intel UHD. This is a non-reference PC. These numbers do not prove the PRD gates.

## Summary

- WIC has no WebP encoder on Windows 10 or in the Windows 11 SDK headers. The PRD says WIC encodes WebP. That is wrong. We must bundle a WebP encoder. (proof: docs/research/proofs/imaging/wic_codecs.cs, winrt_codec_roundtrip.ps1) [1]
- HEIC decode and HEIC encode both fail through WIC when the HEVC Video Extensions are missing. The error is `0xC00D5212` (no suitable transform). The HEIF Image Extension alone is not enough. AVIF decode works when the AV1 extension is present. (proof: winrt_codec_roundtrip.ps1)
- `MFTEnumEx` reliably detects whether an HEVC decoder is installed. It returned 0 HEVC decoders on this PC, and it found the Store-installed AV1 decoder. Use this check, plus a trial decode, to choose between the Windows codec and the libheif fallback. (proof: wic_codecs.cs)
- Background removal: 3 permissive models ship official ONNX files under 500 MB: BiRefNet lite (MIT, 224 MB), BEN2 Base (MIT, 223 MB), and ormbg (Apache-2.0, 176 MB) [2][4][5][7]. Reject BRIA RMBG, withoutBG Open Weights, and RobustVideoMatting because of their licenses [9][10][11][18].
- The ONNX Runtime 1.24.4 DirectML build needs about 39.6 MB of DLLs, uncompressed. That alone exceeds the 30 MB installer budget. The runtime and the model (176 to 224 MB) must both be a first-use download [24].

## Findings

### 1. WIC codec coverage, decode and encode

Measured on this PC. The Store packages installed here are WebP Image Extension 1.2.31.0, HEIF Image Extension 1.2.48.0, AV1 Video Extension 2.0.35.0, and VP9 Video Extensions 1.2.20.0. The HEVC Video Extensions are not installed. (proof: `Get-AppxPackage`, output below)

| Format | WIC decode (Win 10 22H2, this PC) | WIC encode (this PC) | Note |
| --- | --- | --- | --- |
| JPEG | Built in. Decode OK | Built in. Encode OK, 412,305 bytes | (proof: winrt_codec_roundtrip.ps1) |
| PNG | Built in. Decode OK | Built in. Encode OK | Same proof |
| TIFF | Built in | Built in. Encode OK | Same proof |
| GIF | Built in | Built in. Encode OK | Same proof |
| WebP | Decoder listed. Decode OK with the WebP extension installed | **None.** `CreateEncoder(GUID_ContainerFormatWebp)` returns `0x88982F50` (component not found) | The SDK header defines `CLSID_WICWebpDecoder` but no WebP encoder CLSID [1]. WinRT `BitmapEncoder` has no `WebpEncoderId` (proof: winrt_codec_roundtrip.ps1) |
| HEIC (HEVC) | Decoder listed. Decode **fails**, `0xC00D5212`, because no HEVC decoder is installed | Encoder listed and created. Encode **fails**, `0xC00D5212` | Needs the HEVC Video Extensions (proof: winrt_codec_roundtrip.ps1) |
| AVIF | Decode OK through the HEIF decoder with the AV1 extension | Not tested | (proof: winrt_codec_roundtrip.ps1) |
| JPEG XL | Not on this PC | Not on this PC | The SDK defines `CLSID_WICJpegXLDecoder` and `CLSID_WICJpegXLEncoder` [1]. Which Windows 11 builds ship them is unverified |

WIC lists the HEIF and WebP decoders as "built-in" even on Windows 10. With `WICComponentEnumerateBuiltInOnly`, they appear in the built-in list. They are in-box stubs that call the Store codec. The HEIC failure proves that a listed decoder is not a working decoder. (proof: wic_codecs.cs)

Which extensions Windows 11 preinstalls (WebP, HEIF, and the OEM HEVC package) is unverified. The behavior of WebP decode when the WebP extension is missing is also unverified, because this PC has it installed.

Best permissive WebP encoder: libwebp, from Google (BSD-3-Clause, unverified this session).

`wic_codecs.exe` output (decoders and encoders trimmed to the formats in scope):

```
built-in  JPEG Decoder            .jpeg,.jpe,.jpg,.jfif,.exif
built-in  PNG Decoder             .png
built-in  TIFF Decoder            .tiff,.tif
built-in  GIF Decoder             .gif
built-in  Microsoft HEIF Decoder  .heic,.heif,.hif,.avci,.heics,.heifs,.avcs,.avif,.avifs
built-in  Microsoft Webp Decoder  .webp
== WIC encoders
built-in  BMP, GIF, JPEG, PNG, TIFF, WMPhoto, DDS Encoders
built-in  Microsoft HEIF Encoder  .heic,.heif,.hif
== CreateEncoder by container format
WebP: hr=0x88982F50 no encoder
HEIF: hr=0x00000000 encoder available
== Media Foundation video decoders (MFTEnumEx, MFT_ENUM_FLAG_ALL)
HEVC: hr=0x00000000 count=0
AV1 : hr=0x00000000 count=1
   AV1VideoExtension
```

`winrt_codec_roundtrip.ps1` output (decode and encode sections):

```
sample.png             OK   1600x1200
sample.webp            OK   1600x1200
libheif_example.heic   FAIL 0xC00D5212 No suitable transform was found to encode or decode the content.
libheif_example.avif   OK   800x533
out.jpg   OK   412305 bytes
out.png   OK   3135867 bytes
out.tif   OK   3558528 bytes
out.gif   OK   1162109 bytes
out.heic  FAIL 0xC00D5212 No suitable transform was found to encode or decode the content.
WebpEncoderId property exists: False
```

### 2. Fast decode at display size

Not measured. The budget ran out before the timing run. The interop for the measurement is ready: `WicInterop.cs` declares `IWICBitmapSourceTransform` (`GetClosestSize`, `CopyPixels` with a target size), `IWICBitmapScaler`, `GetThumbnail`, `GetColorContexts`, and the metadata query reader for EXIF orientation (`/app1/ifd/{ushort=274}`). The vtable order is copied from wincodec.h [1]. The 24 MP test JPEG is downloaded: "Filband mazandaran Province 10.jpg", 6000x4000, 10.85 MB, CC0 [26].

These claims come from training data, not from this session. They are all unverified:

- The WIC JPEG decoder supports DCT scaling to 1/2, 1/4, and 1/8 through `IWICBitmapSourceTransform` (unverified).
- WIC does not apply EXIF orientation. The app must read the orientation tag and rotate the image (unverified).
- WIC does not color-manage by default. Direct2D's color management effect, or `IWICColorTransform`, can apply the embedded profile (unverified).

### 3. HEIC fallback with libheif and libde265

| Item | Finding |
| --- | --- |
| libheif license | The library is LGPL. The sample applications are MIT [22] |
| libde265 license | LGPL-3.0 (unverified) |
| Load as separate DLLs | Dynamic linking to an unmodified LGPL DLL keeps our code closed (unverified; this is a legal question) |
| Prebuilt Windows binaries, x64 and ARM64, and their size | Not researched. Budget ran out |
| Detect the Windows codec | Measured. `MFTEnumEx(MFT_CATEGORY_VIDEO_DECODER, MFT_ENUM_FLAG_ALL, {MFMediaType_Video, MFVideoFormat_HEVC})` returned 0 here. The same call found the Store `AV1VideoExtension`. This shows that the call sees Store codecs. A trial WIC decode confirms the result (proof: wic_codecs.cs) |
| HEIC encode without GPL | x265 is GPL (unverified this session). kvazaar is BSD-3-Clause and libheif has a kvazaar encoder plugin (unverified). The Windows HEIF encoder works only with the HEVC extension installed (proof: winrt_codec_roundtrip.ps1) |
| **Legal flag** | HEVC is covered by several patent pools. Bundling an HEVC decoder (libde265) or encoder (kvazaar) can expose us to royalty claims. The pools and their terms are unverified. The legal agent must review this before v1. Using the Windows HEVC codec moves that exposure to Microsoft and the user |

The HEIC test file is `examples/example.heic` from the libheif repository [23].

### 4. OCR

Not measured. These items are unverified: Windows.Media.Ocr availability on Windows 10 and 11, its maximum image size, and its output (lines, words, and bounding boxes). The same applies to the Windows AI `TextRecognizer` requirements: Copilot+ PCs only, package identity, and a Limited Access Feature unlock.

### 5. Background removal

`ImageForegroundExtractor` (Windows AI): not researched. Availability (Copilot+ PCs only), packaging requirements, and API surface are unverified.

Open models for non-Copilot+ PCs. The license column covers both code and weights. "Official ONNX" means an ONNX file in the authors' GitHub releases or their Hugging Face repo.

| Model | Code license | Weights license | Official ONNX, size | Input | Verdict |
| --- | --- | --- | --- | --- | --- |
| BiRefNet lite (Swin-T), general | MIT [3] | MIT [4] | Yes. `BiRefNet-general-bb_swin_v1_tiny-epoch_232.onnx`, 224.0 MB, GitHub release v1, May 2024 [2] | 1024x1024 (unverified) | **Benchmark first.** The HF weights were updated on 2026-08-29. That update is newer than the ONNX export [4] |
| BiRefNet lite 2K | MIT [3] | MIT [4] | Yes. 331.1 MB [2] | 2560x1440 (unverified) | Candidate if the 1024 input loses edge detail |
| BiRefNet full | MIT [3] | MIT | Yes. 972.7 MB [2] | 1024x1024 | Too large for a first-use download |
| BEN2 Base | MIT [6] | MIT. The model card says the Base model is open source and the full model is commercial [5] | Yes. `BEN2_Base.onnx`, 222.9 MB [5] | 1024x1024 (unverified) | **Benchmark second** |
| ormbg (IS-Net architecture) | No license file found by the GitHub API [8] | Apache-2.0 [7] | Yes. `ormbg.onnx`, 176.2 MB [7] | 1024x1024 (unverified) | Baseline only. The author says it is tuned for humans, struggles on real images, and has had no update since July 2024 [7] |
| U2-Net and U2-Net-p | Apache-2.0 [13] | Apache-2.0 | No. The weights are on Google Drive as `.pth` files [13] | 320x320 (unverified) | Not benchmarked |
| IS-Net / DIS | Apache-2.0 [14] | Apache-2.0 | No. Google Drive [14] | 1024x1024 (unverified) | Not benchmarked |
| MODNet | Apache-2.0 [15] | Apache-2.0 (the README covers pretrained models; unverified) | Export code is in the repo. The file is not on GitHub releases or HF [15] | Portrait only | Not benchmarked. Portraits only |
| InSPyReNet / transparent-background | MIT [16][17] | MIT | No. Google Drive [16][17] | 1024 (unverified) | Not benchmarked |
| withoutbg Focus | Not checked | Apache-2.0 tag. The model card is empty [12] | Yes. 4 files, 318 MB in total [12] | Several inputs | Unclear provenance. Not benchmarked |
| BRIA RMBG-1.4 and 2.0 | n/a | HF license "other". 2.0 is gated [9][10]. The terms are non-commercial (unverified this session) | Yes | 1024 | **Reject.** Not MIT, Apache, or BSD |
| withoutBG Open Weights | n/a | Custom "withoutBG Open Weights" license plus the Meta DINOv3 License [11] | Yes. About 1.5 GB | Several inputs | **Reject.** The license is not permissive, and the files are too large |
| RobustVideoMatting | GPL-3.0 [18] | n/a | n/a | n/a | **Reject.** GPL |
| BackgroundMattingV2, ViTMatte | MIT [19][20] | MIT | n/a | Need a background plate or a trimap (unverified) | Not one-click |

The downloaded files and their SHA-256 hashes are below. They are ready for the benchmark.

```
5600024376f572a557870a5eb0afb1e5961636bef4e1e22132025467d0f03333  birefnet_lite.onnx (BiRefNet-general-bb_swin_v1_tiny-epoch_232.onnx)
22cea62108ff53b7ccc20f7a008bf30494228d84b1687f29ecbe76936a998101  BEN2_Base.onnx
89b47dd4fa46a76e91b06affeb5ec7881894a27792e41f7dfaf69987653f31d3  ormbg.onnx
```

Test set: 12 photos from Wikimedia Commons, each 4000x3000 (12 MP), downloaded to the scratch folder [26]. They include a border collie, a dog portrait, 2 cats, a woman on a bus, a bicycle, a shoe, a bird on a branch, a sunflower, a red car, a teddy bear, and potted plants. Licenses are CC BY or CC BY-SA. Timing and quality were **not measured**.

### 6. DirectML compared with Windows ML

Not researched from primary sources. Microsoft's current guidance is unverified. This includes whether DirectML is in maintenance mode, whether Windows ML (ONNX Runtime with execution providers, Windows App SDK 1.8 or later) is the recommended path, and whether Windows ML supports Windows 10. One fact is measured: the PyPI wheel `onnxruntime-directml` 1.24.4 installs and offers `DmlExecutionProvider` on this Windows 10 PC [24]. (proof: `python -c "import onnxruntime as o; print(o.__version__, o.get_available_providers())"` printed `1.24.4 ['DmlExecutionProvider', 'CPUExecutionProvider']`)

### 7. Size

| Component | Size | Source |
| --- | --- | --- |
| `onnxruntime.dll` (DirectML build 1.24.4) | 21,111,840 bytes | Measured from the wheel [24] |
| `DirectML.dll` | 18,527,776 bytes | Measured from the wheel [24] |
| Runtime total, uncompressed | about 39.6 MB | Sum of the 2 rows above |
| Model, first-use download | 176 MB (ormbg) to 224 MB (BiRefNet lite, BEN2 Base) | [2][5][7] |
| libheif, libde265, libwebp DLLs | Not measured | — |
| WIC codecs | 0 MB. They are part of Windows or of Store extensions | (proof: wic_codecs.cs) |

The compressed size of the runtime was not measured.

## Confidence

| Finding | Confidence | Reason |
| --- | --- | --- |
| No WIC WebP encoder | High | Three independent checks agree: the SDK header, WIC `CreateEncoder`, and WinRT `BitmapEncoder` |
| HEIC decode and encode need the HEVC extension | High | Measured on this PC with a real HEIC file. Windows 11 behavior was not tested |
| `MFTEnumEx` detects the HEVC codec | Medium | It works for the AV1 Store codec. It was not tested with the HEVC extension installed |
| Model licenses and sizes | High for MIT/Apache tags and file sizes (from GitHub and HF metadata). Medium for "weights MIT" | Some weight terms come from model-card tags, not full license texts |
| ORT plus DirectML runtime is about 40 MB | High for the uncompressed size | The native build for the app may differ from the Python wheel |
| Decode speed, OCR, and background-removal speed and quality | None | Not measured |

## Conflicts with the PRD

1. **"Built-in decode and encode for ... WebP" (stack table).** WIC cannot encode WebP. The app needs libwebp or a similar encoder, which adds size and a license notice. (proof: wic_codecs.cs, winrt_codec_roundtrip.ps1)
2. **Task 8, "Export to ... HEIC".** Without the HEVC Video Extensions, WIC cannot encode HEIC. The PRD notes that this extension can cost $0.99. A bundled HEVC encoder brings patent exposure. HEIC export is not possible on every PC unless legal clears a bundled encoder. (proof: winrt_codec_roundtrip.ps1)
3. **"Uses HEIC codec when present."** The HEIF Image Extension can be present while HEIC still fails. Detection must check for the HEVC decoder, not the HEIF extension. (proof: wic_codecs.cs, winrt_codec_roundtrip.ps1)
4. **Installer under 30 MB.** The ORT plus DirectML runtime is about 40 MB uncompressed. The PRD makes only the model a first-use download. The runtime must be one too, unless a CPU-only build is small enough (not measured) [24].
5. **The "optional AI model" is 176 to 224 MB** for permissive models with official ONNX files [2][5][7]. Users on slow links will wait several minutes the first time. Plan this download UX.

## What we should do

1. Change the PRD stack row for images. WIC decodes JPEG, PNG, TIFF, GIF, WebP (with the extension), and HEIC (with the HEVC extension). WIC encodes JPEG, PNG, TIFF, and GIF. Add libwebp for WebP export after the license is verified.
2. Detect HEIC support with `MFTEnumEx` for an HEVC decoder plus a trial WIC decode. Use the libheif fallback only if legal clears the HEVC patent question.
3. Show HEIC export only when the HEVC encoder works. Otherwise disable it and explain why. Ask legal about kvazaar before bundling an encoder.
4. Benchmark BiRefNet lite and BEN2 Base first, with ormbg as the baseline. Use the 12 test photos already in scratch. Test CPU, DirectML on the Intel iGPU, and DirectML on NVIDIA. The Intel iGPU is closest to the reference laptop.
5. Package the ONNX Runtime and DirectML DLLs with the model as one first-use "AI pack" download. Keep both out of the installer.

## Open questions

These items were not measured or verified because the budget ran out:

1. Decode time for the 24 MP JPEG at full size, through the source transform, through the scaler, and from the EXIF thumbnail. The test file and the interop are ready. Does "next image under 50 ms" hold?
2. Windows.Media.Ocr speed and accuracy on a screenshot and a text photo. Availability and limits of Windows.Media.Ocr and the Windows AI `TextRecognizer`.
3. Background-removal time (CPU and DirectML) and quality on the 12 test images. Is under 3 s on a non-AI laptop realistic with 1024x1024 models?
4. `ImageForegroundExtractor` requirements: Copilot+ PCs only, package identity, and Limited Access Feature status.
5. Microsoft's current DirectML compared with Windows ML guidance, Windows ML support on Windows 10, and the Windows ML runtime size.
6. libheif and libde265 prebuilt x64 and ARM64 binaries and their sizes. The libde265 license. The kvazaar license and libheif plugin support.
7. HEVC patent pool terms for bundled decoders and encoders (legal agent).
8. Which codec extensions Windows 11 preinstalls. WebP decode behavior when the WebP extension is missing.
9. The libwebp license and binary size.
10. Rust options for ORT, WIC, and OCR. The coordinator dropped this item.

## Sources

1. Windows SDK 10.0.26100 header `wincodec.h`, local file `C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\um\wincodec.h`
2. BiRefNet GitHub release v1 (ONNX assets), https://github.com/ZhengPeng7/BiRefNet/releases/tag/v1
3. BiRefNet license (MIT), https://github.com/ZhengPeng7/BiRefNet/blob/main/LICENSE
4. BiRefNet_lite on Hugging Face (MIT), https://huggingface.co/ZhengPeng7/BiRefNet_lite
5. BEN2 model card, Hugging Face (MIT, BEN2_Base.onnx), https://huggingface.co/PramaLLC/BEN2
6. BEN2 GitHub (MIT), https://github.com/PramaLLC/BEN2
7. ormbg model card, Hugging Face (Apache-2.0), https://huggingface.co/schirrmacher/ormbg
8. ormbg GitHub, https://github.com/schirrmacher/ormbg
9. BRIA RMBG-1.4, https://huggingface.co/briaai/RMBG-1.4
10. BRIA RMBG-2.0, https://huggingface.co/briaai/RMBG-2.0
11. withoutBG Open Weights license, https://huggingface.co/withoutbg/withoutbg-openweights-onnx/blob/main/LICENSE
12. withoutbg Focus, https://huggingface.co/withoutbg/focus
13. U-2-Net (Apache-2.0), https://github.com/xuebinqin/U-2-Net
14. DIS / IS-Net (Apache-2.0), https://github.com/xuebinqin/DIS
15. MODNet (Apache-2.0), https://github.com/ZHKKKe/MODNet
16. InSPyReNet (MIT), https://github.com/plemeri/InSPyReNet
17. transparent-background (MIT), https://github.com/plemeri/transparent-background
18. RobustVideoMatting (GPL-3.0), https://github.com/PeterL1n/RobustVideoMatting
19. BackgroundMattingV2 (MIT), https://github.com/PeterL1n/BackgroundMattingV2
20. ViTMatte (MIT), https://github.com/hustvl/ViTMatte
21. rembg (MIT), https://github.com/danielgatis/rembg
22. libheif README, license section, https://github.com/strukturag/libheif#license
23. libheif example files, https://github.com/strukturag/libheif/tree/master/examples
24. onnxruntime-directml on PyPI, https://pypi.org/project/onnxruntime-directml/
25. Hugging Face model API (file sizes and license tags), https://huggingface.co/api/models/PramaLLC/BEN2?blobs=true (same pattern for each repo)
26. Wikimedia Commons test images: [24 MP CC0](https://commons.wikimedia.org/wiki/File:Filband_mazandaran_Province_10.jpg), [Border collie, CC BY-SA 4.0](https://commons.wikimedia.org/wiki/File:Border_Collie_noir_et_blanc_de_4_ans_(2).jpg), [Dog portrait, CC BY 2.0](https://commons.wikimedia.org/wiki/File:Portrait_dog.jpg), [Cat, CC BY-SA 4.0](https://commons.wikimedia.org/wiki/File:Cat_Sitting_Down.jpg), [Tuxedo cat, CC BY 3.0](https://commons.wikimedia.org/wiki/File:Tuxedo_patterned_black_and_white_cat_sitting_on_garden_path_-_panoramio_(3204).jpg), [Woman on bus, CC BY-SA 2.0](https://commons.wikimedia.org/wiki/File:Are_we_there_yet%3F_(Woman_on_bus_in_Mexico).jpg), [Bicycle, CC BY-SA 4.0](https://commons.wikimedia.org/wiki/File:Bicycle_from_India_01.jpg), [Shoe, CC BY-SA 4.0](https://commons.wikimedia.org/wiki/File:ECCO_BIOM_running_shoe.jpg), [Whimbrel, CC BY-SA 2.0](https://commons.wikimedia.org/wiki/File:Whimbrel_(Numenius_phaeopus)_perched_on_branch_(15664255220).jpg), [Sunflower, CC BY-SA 3.0](https://commons.wikimedia.org/wiki/File:Sunflower_in_Canada4.JPG), [Red car, CC BY 2.0](https://commons.wikimedia.org/wiki/File:Maple_Street_New_Orleans_sporty_red_car_2012_-_Back.jpg), [Teddy bear, CC BY-SA 2.0](https://commons.wikimedia.org/wiki/File:Dog_The_Teddy_Bear.jpg), [Potted plants, CC BY-SA 4.0](https://commons.wikimedia.org/wiki/File:Potted_Plants_in_CDG_Terminal_2A_Annex.jpg)
