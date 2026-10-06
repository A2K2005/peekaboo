# Licensing and legal

Date: 2026-10-06
Author: Research agent: licensing and legal
Status: draft. Not legal advice. Items marked **Lawyer** need review by a qualified lawyer.

## Summary

- The core stack is permissive and usable: PDFium, qpdf (native or OpenSSL crypto), ONNX Runtime, CsWinRT, the .NET runtime, and CommunityToolkit. Each one needs a notice in a shipped third-party notices file. The Windows App SDK and DirectML are Microsoft licenses with extra terms: our EULA must protect them, and we must indemnify Microsoft. [1][10][13][18][20][21][23]
- HEVC is the biggest risk. Access Advance now runs both big HEVC pools (it took over Via LA's program on 2025-12-15). Its FAQ says downloaded HEVC software "in general" needs a license. Its licensors sued HP, ASUS, Roku, and Snap. Ship v1 on the Windows HEVC codec. Bundle libde265 only after a lawyer signs off. [38][39][40][41][42]
- LGPL-3.0 (libheif, libde265) is usable through separate DLLs. It also needs a source link, license texts, About-box notices, and our own EULA. The Store's default license terms ban reverse engineering, and LGPL-3.0 section 4 forbids that restriction. [27][32][35]
- Background-removal weights: avoid BRIA RMBG 1.4 and 2.0 (non-commercial). BiRefNet, BEN2, and IS-Net weights are trained on DIS5K. That dataset's terms ban commercial use, even after processing. "MIT or Apache weights" is not enough. **Lawyer** [49][50][53][54][55]
- Rust addition: Slint's free proprietary license requires the `AboutSlint` widget in the About dialog or a badge on the download page. I did not check the other Rust and C++ additions. [66][67]

## Findings

### 1. Dependency table

Linking key: **static** = compiled into our binary. **DLL** = separate DLL loaded at run time. **OS** = part of Windows. **Data** = a file we load, not code.
Obligation key: **N** = put the license text and copyright in the notices file. **A** = list it in the About box. **S** = offer source. **E** = our end-user terms must meet the license's conditions.

PDFium dependency versions come from the PDFium `main` branch on 2026-10-06 [2][3]. The pdfium-binaries release `chromium/8086` (2026-10-05) can differ slightly. Its build writes each dependency's license into a `licenses/` folder [8].

| Component | Version checked | License (SPDX) | Source | Linking | Obligations | Verdict |
| --- | --- | --- | --- | --- | --- | --- |
| .NET runtime and NativeAOT (ILCompiler) | 10.0.12 | MIT | [18] | static (AOT) | N, including `THIRD-PARTY-NOTICES.TXT` (92 KB) | OK with conditions |
| Windows App SDK runtime (WinUI 3 binaries, Foundation) | 2.5.1 | LicenseRef-Microsoft-WindowsAppSDK | [10] | DLL / framework package | E: end users must agree to terms that protect it "at least as much as this agreement". We must indemnify Microsoft. No Microsoft trademarks. No copyleft that reaches it. | OK with conditions |
| WinUI 3 source (microsoft-ui-xaml) | main | MIT | [12] | not shipped by us | none (we ship the Windows App SDK binaries) | OK |
| Windows ML (Microsoft.WindowsAppSDK.ML) | 2.1.94 | LicenseRef-Microsoft-WindowsAppSDK-ML | [11] | DLL | same as Windows App SDK | OK with conditions |
| CsWinRT | 2.3.1 | MIT | [13] | static | N | OK |
| Microsoft.Windows.SDK.NET.Ref (WinRT projections) | 10.0.28000.87 | LicenseRef-Microsoft-WindowsSDK | [19] | static / DLL | E. Show our own copyright notice. Indemnify Microsoft. Windows only. | OK with conditions |
| Win2D | 1.4.0 | MIT (source) [14]. The official NuGet points to a separate Microsoft EULA, and that URL returns 403. [15][16] | — | DLL | N. EULA terms (unverified). | OK with conditions: read the EULA in the .nupkg, or build from MIT source |
| CommunityToolkit (Mvvm 8.4.2; WinUI Controls 8.2.251219) | see left | MIT | [17] | static / DLL | N | OK |
| PDFium | chromium/8086 | BSD-3-Clause AND Apache-2.0 (the LICENSE file contains both texts) | [1] | DLL | N: both texts. Apache-2.0 section 4. | OK with conditions |
| pdfium-binaries packaging | chromium/8086 | MIT (repo). The NuGet declares Apache-2.0. | [8][9] | build input | N: copy its `licenses/` folder | OK |
| FreeType (in PDFium) | VER-2-14-3-85 | FTL (dual FTL / GPL-2.0; we take FTL) | [2] | static in pdfium.dll | N, plus a FreeType credit in our docs (exact wording unverified; read `FTL.TXT`) | OK with conditions |
| libjpeg-turbo (in PDFium, qpdf) | 3.1.0 | IJG AND BSD-3-Clause (zlib terms subsumed) | [6] | static | N, plus an IJG credit (exact wording unverified; read `README.ijg`) | OK with conditions |
| OpenJPEG | 2.5.4 | BSD-2-Clause | [2] | static | N | OK |
| Little CMS (lcms2) | 2.19 | MIT | [2] | static | N | OK |
| zlib (PDFium, qpdf) | 1.3.2 | Zlib | [4] | static | N (good practice) | OK |
| libpng | 1.6.58 | Libpng-2.0 | [4] | static, if built | N | OK |
| libtiff | 4.7.2 | libtiff | [2] | static | N | OK |
| AGG (Anti-Grain Geometry) | 2.3 | MIT (per README.pdfium) | [2] | static | N | OK |
| Abseil | DEPS pin | Apache-2.0 | [4] | static | N | OK |
| ICU | 78-2 | Unicode-3.0 (the README.chromium says MIT; the LICENSE file says Unicode-3.0) | [5] | static, only in ICU-enabled builds | N | OK |
| HarfBuzz | 14.5.0 | MIT-Modern-Variant | [2] | static | N | OK |
| Small PDFium deps: bigint, fast_float, dragonbox, fp16, llvm-libc, libc++, simdutf, brotli, libyuv | DEPS pins | Public Domain; MIT; Apache-2.0 WITH LLVM-exception OR BSL-1.0; MIT; Apache-2.0 WITH LLVM-exception; same; MIT; MIT; BSD-3-Clause | [2][4] | static | N | OK |
| V8 (XFA builds only) | DEPS pin | BSD-3-Clause, plus fdlibm and Strongtalk notices | [7][8] | static | N | Avoid in v1. The PRD sends XFA files to Edge, so we do not need V8. |
| qpdf | 12.4.2 | Apache-2.0 (Artistic-2.0 also allowed) | [23] | DLL | N, plus `NOTICE.md`, which carries the sphlib MIT notice and a public-domain Rijndael note | OK with conditions |
| qpdf crypto: native | 12.4.2 | Public domain (Rijndael) + MIT (sphlib) | [23] | static in qpdf | N | OK. Preferred. |
| qpdf crypto: OpenSSL | not checked | Apache-2.0 | [26] | DLL | N | OK with conditions |
| qpdf crypto: GnuTLS | not checked | LGPL-2.1-or-later. Its nettle and gmp dependencies are LGPL-3.0+ or GPL-2.0+. | [25] | DLL | LGPL duties | Avoid |
| libheif | 1.23.6 | LGPL-3.0 (library). The samples are MIT. | [27] | DLL | N, A, S, E (see section 2) | OK with conditions. HEVC patents: **Lawyer**. |
| libde265 | 1.1.3 | LGPL-3.0 | [28] | DLL (libheif plugin or dependency) | N, A, S, E (see section 2) | OK with conditions. HEVC patents: **Lawyer**. |
| kvazaar (HEVC encoder) | 2.3.2 | BSD-3-Clause | [29] | DLL | N | License OK. HEVC patents: **Lawyer**. |
| x265 (HEVC encoder) | not checked | GPL-2.0, or a paid commercial license | [30] | — | GPL | Avoid |
| libwebp | 1.6.0 | BSD-3-Clause, plus Google's WebM patent grant | [31] | DLL, only if we bypass WIC | N | OK |
| ONNX Runtime | 1.30.0 | MIT | [20] | DLL | N, plus `ThirdPartyNotices.txt` (338 KB) | OK with conditions |
| ONNX Runtime DirectML package | 1.24.4 | MIT | [20] | DLL | N | OK. DirectML is in maintenance mode. [22] |
| DirectML redistributable | 1.15.4 | LicenseRef-Microsoft-DirectML | [21] | DLL | Windows and Xbox only. No reverse engineering. Keep the notices. | OK with conditions |
| Windows OCR, Windows AI APIs, WIC, Direct2D | OS | OS component | — | OS | none | OK |
| Model: U2-Net | repo 2024-06 | Apache-2.0 (repo). The weights' license is not stated. The default weights are trained on DUTS-TR. | [48] | Data | N | OK with conditions: dataset terms (unverified) |
| Model: IS-Net / DIS | repo 2024-09 | Apache-2.0 (code and metric). The DIS5K terms ban commercial use "even after ... processing". | [49] | Data | N | Avoid unless a lawyer clears it |
| Model: BiRefNet | HF 2026-02-04 | MIT (code and weights). The DIS weights are trained on DIS5K-TR. | [50] | Data | N | OK with conditions: dataset provenance, **Lawyer** |
| Model: InSPyReNet | repo 2025-05 | MIT (code). Some checkpoints are trained on DIS5K. | [51] | Data | N | OK with conditions: dataset provenance (unverified) |
| Model: MODNet | repo 2024-05 | Apache-2.0 ("code, models, and demos") | [52] | Data | N | License OK. Portrait matting only, so a poor fit. |
| Model: BEN2 (base) | HF 2025-12-31 | MIT. Trained on DIS5K plus a 22K proprietary set. The full model is commercial and API-only. | [53] | Data | N | OK with conditions: dataset provenance, **Lawyer** |
| Model: BRIA RMBG 1.4 | HF 2025-07-06 | LicenseRef-bria-rmbg-1.4: "source-available ... for non-commercial use" | [54] | Data | — | Avoid |
| Model: BRIA RMBG 2.0 | HF 2026-04-06 | CC-BY-NC-4.0 (per the license link) | [55] | Data | — | Avoid |
| Font: Segoe UI Variable | OS | Windows font. Redistribution is "generally not allowed". | [58][59] | OS | Do not bundle | OK on Windows 11. Windows 10 behavior unverified. |
| Icons: Segoe Fluent Icons | OS | Windows font. Not included on Windows 10. The download is "for use in design and development". | [56][59] | OS | Do not bundle | OK on Windows 11 only |
| Icons: Segoe MDL2 Assets | OS | Windows font | [57] | OS | Do not bundle | OK (Windows 10 fallback) |
| Icons: Fluent UI System Icons | main | MIT | [60] | Data (font or SVG) | N | OK. Safe to bundle for Windows 10. |
| Reference: SumatraPDF | main | GPL-3.0 | [61] | none | — | Study only. Never copy. |
| Reference: MuPDF | main | AGPL-3.0 | [62] | none | — | Avoid |
| Reference: QuickLook | main | GPL-3.0 | [63] | none | — | Study only. Never copy. |
| Reference: ImageGlass | main | GPL-3.0 (source). Store binaries use separate paid terms. | [64] | none | — | Study only. Never copy. |
| Reference: PDF Arranger | main | GPL-3.0 | [65] | none | — | Study only. Never copy. |
| Slint (Rust UI) | 1.18.1 | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0 | [66][67] | static | Royalty-free 2.0: show the `AboutSlint` widget in an About dialog reached from the top-level menu, or show the Slint badge on a public download page. The app must not expose Slint's APIs. Embedded systems are excluded. | OK with conditions (attribution) |
| Rust: windows-rs (windows, windows-sys), pdfium-render, ort, image-rs, AccessKit | not checked | (unverified) | — | static | (unverified) | Check before use |
| Rust UI: egui/eframe, iced, gpui, Xilem/Masonry | not checked | (unverified). gpui lives in the Zed repo, which mixes licenses; check that gpui itself is not GPL or AGPL. (unverified) | — | static | (unverified) | Check before use |
| Rust standard library and runtime notice duties | not checked | (unverified) | — | static | (unverified) | Check before use |
| C++/WinRT, WIL | not checked | (unverified) | — | static | (unverified) | Check before use |
| Avalonia, SkiaSharp (Skia) | not checked | (unverified) | — | DLL | (unverified) | Check before use |

### 2. LGPL-3.0 in practice: libheif and libde265 in a signed MSIX

What the license requires for a closed app (a "Combined Work"):

1. Our terms, "taken together", must not restrict modification of the library portions, or "reverse engineering for debugging such modifications". [32] (LGPL-3.0 section 4)
2. Prominent notice that the library is used and is under the LGPL. [32] (4a)
3. A copy of the GPL-3.0 and LGPL-3.0 texts. [32] (4b)
4. If the app shows copyright notices while it runs, it must show the library's notice too, with a pointer to the license texts. [32] (4c)
5. Either ship relinkable application code (4d0) or "use a suitable shared library mechanism". That mechanism must use a copy of the library "already present on the user's computer system" at run time. It must also "operate properly with a modified version of the Library that is interface-compatible". [32] (4d1) Separate DLLs give us 4d1.
6. The library itself is object code that we convey. So we must offer its Corresponding Source, for example on a server, with "clear directions next to the object code saying where to find" it. [33] (GPL-3.0 section 6d)

**Relinking under a signed MSIX.** Windows installs MSIX packages to a protected, read-only folder. A user cannot swap a DLL there without repackaging and re-signing. (unverified: I found no primary source for the folder protection in this pass.) Whether that still meets 4d1(b) is open. **Lawyer** The safe option removes the question: let the user point the app at their own interface-compatible `libheif.dll` / `libde265.dll` in a user folder. libheif already reads the `LIBHEIF_PLUGIN_PATH` environment variable for codec plugins. [27]

**"Installation Information" (4e).** LGPL 4e applies only "if you would otherwise be required to provide such information under section 6 of the GNU GPL". [32] GPL-3.0 section 6 requires it only when object code is conveyed "in, or with, or specifically for use in, a User Product" and "the conveying occurs as part of a transaction in which the right of possession and use of the User Product is transferred". [33] A Store or winget download to a PC the user already owns does not transfer a User Product. So Installation Information is likely not required. **Lawyer** to confirm. It would apply if an OEM ever preloads the app on new PCs.

**Store terms conflict.** If we give no license, the Store applies its Standard Application License Terms (SALT). [35] SALT bans reverse engineering and bans making the app "available for others to copy". [35] That conflicts with LGPL section 4. The App Developer Agreement lets us supply our own license. [35] The Windows App SDK license requires our end-user terms to protect it "at least as much as this agreement". [10] Our EULA must therefore do both: grant the LGPL rights for the LGPL parts only, and protect the Microsoft parts. **Lawyer**

**Safest compliant setup:**

1. Build unmodified upstream libheif and libde265 as separate DLLs. If we patch them, publish the patches.
2. Load them at run time with `LoadLibrary`, only when a HEIC file needs the fallback.
3. Add an opt-in setting: "Use a custom HEIF decoder", which loads the DLLs from a user folder. Show a warning, because loading DLLs from a user-writable folder is a code-injection path.
4. Host the exact source archives and build scripts on our public site or repo. Link to them from the About box, the notices file, and the Store description. Keep each version online as long as we ship it.
5. Ship `COPYING` (LGPL-3.0 and GPL-3.0 texts). List libheif and libde265 with copyright lines in the About box.
6. Publish our own EULA in Partner Center, with an LGPL carve-out.
7. Optional: decode HEIC in a separate low-privilege process. It isolates the LGPL code. It also limits damage from parser bugs: libheif published 73 security advisories between January and October 2026. [27]

FSF sources: gnu.org was unreachable (connection refused) during this research. I quoted the license texts from the verbatim copy in libheif's `COPYING` [27] and cite the FSF URLs [32][33]. I did not read the FSF GPL FAQ. (unverified)

### 3. HEVC patents

**Who licenses HEVC decode patents.**

- Access Advance runs the HEVC Advance pool. In 2024 it said the pool held about 75 to 80% of HEVC-essential patents, with 43 licensors and more than 320 licensees, Microsoft among them. [41]
- Via LA's HEVC/VVC program: "As of December 15, 2025, Access Advance has acquired this HEVC/VVC program." [40] It is now the VCL Advance pool. [39] The Via LA rate is $0.00 for units 1 to 100,000 per year, then $0.30 (Region 1) or $0.20 (Region 2), capped at $30M. The page says the license "extends to devices implementing the technology". [40]
- HEVC Advance rates "as of July 1, 2026" list "All HEVC Software" at $1.00 (Region 1) and $0.50 (Region 2) per copy. [37] The program licenses parties that "Sell" devices or software with HEVC decoders or encoders. [37] Whether free distribution counts as "Sell" depends on license definitions I did not read. (unverified)
- Some HEVC patent holders sit outside these pools. Who they are now: (unverified).

**Does a free app with libde265 owe royalties?** Unclear. **Lawyer**

- Access Advance FAQ: "In general, HEVC software downloaded by users requires a license. However, there are some situations wherein a license is not needed." It asks developers to contact it. [38]
- In November 2016, HEVC Advance said it would not seek royalties on application-layer software downloaded to PCs or phones after sale, decoding fully in software on a general-purpose CPU. [44] That source is secondary press coverage. I could not confirm the policy is still in force. (unverified) libde265 is that kind of software.
- The pool's own fine print: Access Advance "reserves the right to seek licenses from any party infringing". [37]

**Known enforcement.**

- Pool licensors sued HP and ASUS in 2024. [41]
- They sued Roku in 2024 (United States), in October 2025 (Brazil), and won a German preliminary injunction in November 2025. [41][43]
- In March 2026, Dolby, a pool licensor, sued Snap Inc. for AV1 and HEVC infringement. [42] That shows app and streaming companies are targets too.
- I found no public case against a free image viewer that bundles libde265. Absence of evidence is not safety. (unverified)

**Alternatives.**

1. Rely on Windows codecs. WIC with the HEIF Image Extension (free) [47] and HEVC Video Extensions ($0.99) [45]. Microsoft is a pool licensee [41]. Many OEM PCs ship "HEVC Video Extensions from Device Manufacturer" (free listing) [46]. Who can install that listing: (unverified).
2. When the codec is missing, show a one-click Store prompt (an `ms-windows-store://pdp/?ProductId=9NMZLZ57R3T7` deep link). Store policy allows acquiring add-ons or extensions "that enhance the functionality of the product", with user consent. [34] (10.1.5, 10.2.3)
3. Bundle libde265 only after a lawyer's opinion, or written confirmation from Access Advance, or a pool license.
4. HEIC export (PRD task 8) needs an HEVC encoder. x265 is GPL. kvazaar is BSD but has the same patent exposure. Export HEIC only through the Windows codec. Whether WIC's HEIF encoder works with the Store extension: (unverified).

### 4. Microsoft Store policy points

Store Policies version 7.20, effective 2026-10-22 [34]. App Developer Agreement version 8.11, effective 2026-04-17 [35].

1. **Open-source licenses.** I found no clause that bans GPL or LGPL apps. The agreement makes us "solely responsible for ... compliance with those license terms and conditions including any source code availability requirements". [35]
2. **Our own EULA.** We may give customers our own license. Otherwise SALT applies. [35] We need our own EULA for LGPL (section 2).
3. **Model downloads after install.** Policy 10.2.2 bans changing or extending "described functionality ... through any form of dynamic inclusion of code". [34] A model file is data that runs inside our described feature. Describe "background removal; model downloads on first use" in the listing. Host the model ourselves, so its license file and hash stay under our control.
4. **Codec dependency.** Under 10.2.4, a dependency on other software for primary functionality must be disclosed "at the beginning of the description". [34] HEIC is not our primary function, but disclose the Windows codec need anyway. 10.1.5 and 10.2.3 allow offering the HEVC extension because it enhances the app. [34] The 10.2.1 codec rule applies to browsers only. [34]
5. **Privacy policy.** "Desktop Bridge and Win32 products" must always have one. [34] (10.5.1) A packaged WinUI 3 desktop app likely falls in that group (unverified). The model download and opt-in telemetry need one anyway.
6. **Content rights.** All content must be "appropriately licensed". [34] (11.2) That covers models, icons, and fonts.
7. **winget.** I did not check the winget-pkgs policies. (unverified)

### 5. Third-party notices plan

Ship three things:

1. `ThirdPartyNotices.txt` in the package root: one section per component with name, version, license ID, copyright, full license text, and source URL. Append the upstream notice files verbatim: the .NET `THIRD-PARTY-NOTICES.TXT` [18], the ONNX Runtime `ThirdPartyNotices.txt` [20], qpdf `NOTICE.md` [23], and the pdfium-binaries `licenses/` folder [8].
2. `licenses/` with the full texts that must travel with the binary: GPL-3.0, LGPL-3.0, Apache-2.0, FTL, IJG, and the Microsoft redistributable terms.
3. About > "Open-source licenses": shows the file, the libheif and libde265 notices with the source link, the FreeType and IJG credits, and `AboutSlint` if we pick Slint.

Generate it in CI with one script:

1. NuGet: read `<license>` from each resolved package's `.nuspec` (the packages lock file lists them). Copy the license file from the package.
2. Native: a small hand-kept manifest lists each native dependency, its version, and its license file paths. There are about six native packages.
3. Models: the manifest lists each model's card license and its training datasets.
4. Fail the build if a dependency has no license entry, or matches a deny list: `GPL-*`, `AGPL-*`, `LGPL-*` (except allow-listed libheif and libde265), `CC-BY-NC-*`, or `other` without review.

### 6. Red flags

| Item | Why |
| --- | --- |
| MuPDF | AGPL-3.0. [62] |
| SumatraPDF, QuickLook, PDF Arranger, ImageGlass | GPL-3.0. [61][63][64][65] Read for patterns only. |
| x265 | GPL-2.0, or a paid license. [30] |
| GnuTLS (qpdf option) | LGPL-2.1+. Its dependencies are LGPL-3.0+ or GPL-2.0+. [25] |
| libheif, libde265 | LGPL-3.0 duties (section 2). [27][28] 73 security advisories in 2026. [27] |
| Any HEVC decoder or encoder (libde265, kvazaar, x265) | HEVC patents (section 3). |
| BRIA RMBG 1.4 and 2.0 | Non-commercial licenses. [54][55] |
| Weights trained on DIS5K (IS-Net, BiRefNet DIS, BEN2, some InSPyReNet checkpoints) | DIS5K terms ban commercial use "even after copying, editing, processing". [49] |
| Slint (if chosen) | GPL-3.0 option, or a mandatory visible attribution under the free proprietary license. [66][67] |
| FreeType | Dual FTL / GPL-2.0. Use the FTL option and its credit. [2] |
| Win2D official NuGet | Separate Microsoft EULA. Its URL returns 403. [15][16] |
| Segoe Fluent Icons, Segoe UI Variable | Must not be bundled. Fluent Icons is not on Windows 10. [56][59] |
| Windows App SDK, Windows SDK | Indemnity, and a requirement that end-user terms protect Microsoft. [10][19] |

## Confidence

| Finding | Confidence | Reason |
| --- | --- | --- |
| License IDs in the table (verified rows) | High | Read from LICENSE files, README.pdfium/README.chromium metadata, NuGet nuspecs, or the GitHub license API. |
| PDFium dependency versions | Medium | Taken from `main` on 2026-10-06, not from the exact `chromium/8086` build. |
| LGPL-3.0 duties | High on the text, medium on MSIX relinking | Verbatim license text. How 4d1 applies to a signed MSIX is untested. gnu.org was unreachable. |
| Installation Information not required | Medium | Plain reading of GPL-3.0 section 6. No case law checked. |
| HEVC: free app may need a license | Medium | The primary FAQ is explicit that it is unclear. The 2016 exemption is secondary and unverified. |
| HEVC enforcement list | High | Access Advance press releases. |
| Model weight risk from training data | Medium | Dataset terms are clear. Whether they reach trained weights is a legal question. |
| Store policy points | High on the text, medium on how certification applies it | Current policy and agreement texts. |
| Win2D EULA | Low | The EULA page returns 403. |
| Rust, C++, Avalonia additions (except Slint) | None | Not checked, by instruction. |

## Conflicts with the PRD

1. **PDFium "Apache 2.0".** The LICENSE file holds both BSD-3-Clause and Apache-2.0 texts. [1] PDFium bundles about 20 libraries with their own notices, including FreeType and IJG credits. [2][6] "No source-release duty" is correct, but there are notice duties.
2. **WinUI 3 "MIT".** The source repo is MIT. [12] We ship Windows App SDK binaries under Microsoft terms that require end-user terms protecting Microsoft, plus an indemnity. [10]
3. **Win2D "MIT".** The source is MIT. [14] Microsoft's signed NuGet uses a separate EULA, which we have not read. [15][16]
4. **HEIC fallback "LGPL-3.0 (dynamic linking keeps our code closed)".** True but incomplete. It also needs a source link, license texts, About notices, our own EULA (SALT conflicts), and a relinking answer under MSIX. [27][32][35]
5. **HEIC "otherwise bundled decoder, so iPhone photos open with no Store purchase".** This puts HEVC patent risk in v1 by default. Access Advance says downloaded HEVC software "in general" needs a license. [38] The PRD's mitigation ("legal review before v1") stands, but the default should flip to Windows codecs.
6. **Task 8 "Export to ... HEIC".** It needs an HEVC encoder. The PRD names none. x265 is GPL. All encoders carry HEVC patent risk. [29][30]
7. **Background removal "MIT or Apache only".** The weights' license is not enough. Training-data terms (DIS5K) can bar commercial use. [49]
8. **"Segoe UI Variable" theme, with Windows 10 in scope.** Segoe Fluent Icons is not on Windows 10, and Windows fonts must not be bundled. [56][59] Use Fluent UI System Icons (MIT) [60] or Segoe MDL2 Assets [57] on Windows 10.
9. **ImageGlass "open source, .NET".** It is GPL-3.0. [64] Study only.
10. **"ONNX Runtime with DirectML".** DirectML is in maintenance mode. Microsoft points to Windows ML on Windows 11 24H2 and later. [22] This is a stack risk, not a license problem.

## What we should do

1. Use the pdfium-binaries non-V8 build. Copy its `licenses/` folder into our notices. [8]
2. Build qpdf with native crypto. Never enable GnuTLS. [23][25]
3. v1 HEIC: use Windows codecs only, and show a Store prompt for HEVC Video Extensions. Put bundled libde265 behind a lawyer's sign-off. If cleared, follow the section 2 setup.
4. HEIC export: Windows codec only. No x265.
5. Background removal: drop BRIA and IS-Net. Ask a lawyer to compare BiRefNet (MIT weights, DIS5K data) with U2-Net (Apache repo, DUTS-TR data). Prefer the Windows AI foreground extractor where the OS has it.
6. Write our own EULA with an LGPL carve-out and Windows App SDK protection. Publish it in Partner Center. Publish a privacy policy.
7. Add the notices generator and license deny list to CI before the first dependency lands.
8. Bundle Fluent UI System Icons (MIT) for Windows 10, or fall back to Segoe MDL2 Assets.
9. Read the license file inside the Win2D `.nupkg`, or build Win2D from MIT source.
10. If Slint is picked, use its Royalty-free 2.0 license and put `AboutSlint` in the About dialog.
11. Check licenses for the unverified Rust, C++, and Avalonia additions before Phase 2 picks a stack.

## Open questions

Lawyer:

1. Does free distribution of an app that bundles libde265 need an HEVC Advance or VCL Advance license? Is the 2016 software policy in force? Which non-pool HEVC holders matter?
2. Does a signed, read-only MSIX meet LGPL-3.0 section 4d1? Do we need the user override folder?
3. Confirm that the GPL-3.0 "Installation Information" clause does not reach a Store or winget download.
4. Draft the EULA: the LGPL reverse-engineering carve-out versus the Windows App SDK "protect at least as much" clause.
5. Do the DIS5K terms (and ImageNet-pretrained backbones) restrict our use of MIT-licensed weights in a free app published by a company?
6. Is a free app published by a company "commercial use" under non-commercial terms? This is moot if we avoid non-commercial items.

Non-lawyer:

7. What are the Win2D NuGet EULA terms?
8. What do Segoe UI Variable and WinUI 3 fonts do on Windows 10?
9. Do the winget-pkgs policies add any duties?
10. Is the MSIX install folder protection documented? Does certification treat a WinUI 3 app as "Desktop Bridge" for the privacy policy rule?
11. What are the licenses of the Rust, C++, and Avalonia additions?

## Sources

1. PDFium LICENSE — https://pdfium.googlesource.com/pdfium/+/refs/heads/main/LICENSE
2. PDFium third_party README.pdfium files — https://pdfium.googlesource.com/pdfium/+/refs/heads/main/third_party/
3. PDFium DEPS — https://pdfium.googlesource.com/pdfium/+/refs/heads/main/DEPS
4. Chromium third_party README.chromium (abseil-cpp, libpng, zlib, brotli, simdutf, libc++) — https://chromium.googlesource.com/chromium/src/+/main/third_party/
5. ICU LICENSE (Chromium deps) — https://chromium.googlesource.com/chromium/deps/icu/+/refs/heads/main/LICENSE
6. libjpeg-turbo LICENSE.md and README.chromium (Chromium deps) — https://chromium.googlesource.com/chromium/deps/libjpeg_turbo/+/refs/heads/main/LICENSE.md
7. V8 LICENSE — https://chromium.googlesource.com/v8/v8/+/refs/heads/main/LICENSE
8. bblanchon/pdfium-binaries (LICENSE, steps/05-configure.sh, steps/08-licenses.sh) — https://github.com/bblanchon/pdfium-binaries
9. NuGet bblanchon.PDFium.Win32 157.0.8086 — https://www.nuget.org/packages/bblanchon.PDFium.Win32
10. Microsoft Software License Terms, Windows App SDK (NuGet 2.5.1) — https://www.nuget.org/packages/Microsoft.WindowsAppSDK/2.5.1/License
11. Microsoft Software License Terms, Windows App SDK and Windows ML (NuGet 2.1.94) — https://www.nuget.org/packages/Microsoft.WindowsAppSDK.ML/2.1.94/License
12. microsoft/microsoft-ui-xaml repository (MIT) — https://github.com/microsoft/microsoft-ui-xaml
13. CsWinRT license (NuGet 2.3.1) — https://www.nuget.org/packages/Microsoft.Windows.CsWinRT/2.3.1/License
14. Win2D LICENSE.txt — https://github.com/microsoft/Win2D/blob/HEAD/LICENSE.txt
15. Win2D build-nupkg.cmd (signed packages use a Microsoft EULA) — https://github.com/microsoft/Win2D/blob/HEAD/build/nuget/build-nupkg.cmd
16. NuGet Microsoft.Graphics.Win2D 1.4.0 (licenseUrl http://www.microsoft.com/web/webpi/eula/eula_win2d_10012014.htm, returned 403) — https://www.nuget.org/packages/Microsoft.Graphics.Win2D/1.4.0
17. Windows Community Toolkit License.md — https://github.com/CommunityToolkit/Windows/blob/HEAD/License.md ; .NET Community Toolkit License.md — https://github.com/CommunityToolkit/dotnet/blob/HEAD/License.md
18. dotnet/runtime LICENSE.TXT and THIRD-PARTY-NOTICES.TXT — https://github.com/dotnet/runtime ; NuGet Microsoft.DotNet.ILCompiler 10.0.12 — https://www.nuget.org/packages/Microsoft.DotNet.ILCompiler
19. Windows SDK license (from Microsoft.Windows.SDK.NET.Ref licenseUrl) — https://aka.ms/WinSDKLicenseURL
20. ONNX Runtime license (NuGet 1.30.0) — https://www.nuget.org/packages/Microsoft.ML.OnnxRuntime/1.30.0/License ; ThirdPartyNotices — https://github.com/microsoft/onnxruntime/blob/main/ThirdPartyNotices.txt
21. DirectML license (NuGet 1.15.4) — https://www.nuget.org/packages/Microsoft.AI.DirectML/1.15.4/License
22. DirectML README ("maintenance mode") — https://github.com/microsoft/DirectML
23. qpdf README.md and NOTICE.md — https://github.com/qpdf/qpdf
24. qpdf README-windows.md — https://github.com/qpdf/qpdf/blob/main/README-windows.md
25. GnuTLS README, Licensing section — https://github.com/gnutls/gnutls
26. OpenSSL repository (Apache-2.0) — https://github.com/openssl/openssl
27. libheif COPYING and README (license, plugins, funding, security) — https://github.com/strukturag/libheif
28. libde265 COPYING — https://github.com/strukturag/libde265
29. kvazaar LICENSE and README — https://github.com/ultravideo/kvazaar
30. x265 COPYING and readme.rst — https://bitbucket.org/multicoreware/x265_git
31. libwebp (BSD-3-Clause, PATENTS) — https://github.com/webmproject/libwebp
32. GNU LGPL v3 — https://www.gnu.org/licenses/lgpl-3.0.html (text verified from the verbatim copy in [27])
33. GNU GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html (text verified from the verbatim copy in [27])
34. Microsoft Store Policies, version 7.20 — https://learn.microsoft.com/en-us/windows/apps/publish/store-policies
35. Microsoft Store App Developer Agreement, version 8.11 — https://learn.microsoft.com/en-us/legal/windows/agreements/app-developer-agreement
36. Access Advance, HEVC Advance program — https://accessadvance.com/licensing-programs/hevc-advance/
37. HEVC Advance Program Overview (July 2026) — https://accessadvance.com/wp-content/uploads/2025/07/HEVC-Advance-Program-Overview-July-2026.pdf
38. Access Advance FAQ — https://accessadvance.com/faq/
39. Access Advance, VCL Advance pool — https://accessadvance.com/licensing-programs/vcl-advance/
40. Via LA, HEVC/VVC program — https://www.via-la.com/licensing/hevc-vvc
41. Access Advance, "HEVC Licensors Seek Level Playing Field" (2024-07-03) — https://accessadvance.com/2024/07/03/access-advance-hevc-licensors-seek-level-playing-field-by-bringing-patent-infringement-suits/
42. Access Advance, "Licensor Sues Snap Inc." (2026-03-24) — https://accessadvance.com/2026/03/24/access-advance-licensor-sues-snap-inc-for-av1-and-hevc-patent-infringement/
43. Access Advance litigation news list — https://accessadvance.com/category/litigation/
44. Streaming Learning Center, "HEVC Advance Makes Some Software Royalty Free" (secondary; 2016 policy) — https://streaminglearningcenter.com/blogs/hevc-advance-makes-software-royalty-free.html
45. Microsoft Store, HEVC Video Extensions — https://apps.microsoft.com/detail/9NMZLZ57R3T7
46. Microsoft Store, HEVC Video Extensions from Device Manufacturer — https://apps.microsoft.com/detail/9N4WGH0Z6VHQ
47. Microsoft Store, HEIF Image Extension — https://apps.microsoft.com/detail/9PMMSR1CGPWG
48. U-2-Net repository — https://github.com/xuebinqin/U-2-Net
49. DIS repository and DIS5K Dataset Terms of Use — https://github.com/xuebinqin/DIS ; https://github.com/xuebinqin/DIS/blob/main/DIS5K-Dataset-Terms-of-Use.pdf
50. BiRefNet repository and model card — https://github.com/ZhengPeng7/BiRefNet ; https://huggingface.co/ZhengPeng7/BiRefNet
51. InSPyReNet repository — https://github.com/plemeri/InSPyReNet
52. MODNet repository, License section — https://github.com/ZHKKKe/MODNet
53. BEN2 repository and model card — https://github.com/PramaLLC/BEN2 ; https://huggingface.co/PramaLLC/BEN2
54. BRIA RMBG-1.4 model card — https://huggingface.co/briaai/RMBG-1.4
55. BRIA RMBG-2.0 model card — https://huggingface.co/briaai/RMBG-2.0
56. Segoe Fluent Icons font — https://learn.microsoft.com/en-us/windows/apps/design/style/segoe-fluent-icons-font
57. Segoe MDL2 Assets — https://learn.microsoft.com/en-us/windows/apps/design/style/segoe-ui-symbol-font
58. Typography in Windows (Segoe UI Variable) — https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/typography
59. Font redistribution FAQ for Windows — https://learn.microsoft.com/en-us/typography/fonts/font-faq
60. Fluent UI System Icons (MIT) — https://github.com/microsoft/fluentui-system-icons
61. SumatraPDF repository (GPL-3.0) — https://github.com/sumatrapdfreader/sumatrapdf
62. MuPDF repository (AGPL-3.0) — https://github.com/ArtifexSoftware/mupdf
63. QuickLook repository (GPL-3.0) — https://github.com/QL-Win/QuickLook
64. ImageGlass LICENSE — https://github.com/d2phap/ImageGlass/blob/HEAD/LICENSE
65. PDF Arranger repository (GPL-3.0) — https://github.com/pdfarranger/pdfarranger
66. Slint LICENSE.md — https://github.com/slint-ui/slint/blob/HEAD/LICENSE.md
67. Slint Royalty-free License 2.0 — https://github.com/slint-ui/slint/blob/HEAD/LICENSES/LicenseRef-Slint-Royalty-free-2.0.md
