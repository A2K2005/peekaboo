# PDF engine: PDFium coverage, saving, encryption, and interop

Date: 2026-10-06
Author: Research agent: PDF engine
Status: draft

## Summary

- PDFium covers 15 of the 18 v1 PDF tasks with public C APIs, proved by code. It does not create Line (arrow), Polygon, Polyline, Caret, or Redact annotations. Merged pages lose their form fields. Copied text follows content-stream order, not visual reading order. (proof: docs/research/proofs/pdfium/coverage_probe.py)
- `FPDF_INCREMENTAL` keeps the original bytes, but it appends every object PDFium loaded in the session. One highlight added after viewing all 500 pages appended 52 MB to a 52 MB file. Saving from a fresh document handle appended 109 KB in 104 ms. (proof: docs/research/proofs/pdfium/incremental_save.py) [3]
- PDFium cannot add encryption. It keeps existing encryption on save. qpdf 12.4.2 (Apache-2.0) adds AES-256 through its CLI and its C API; it encrypted a 52 MB file in 1.2 s. (proof: docs/research/proofs/pdfium/encrypt_gap.py) [6]
- PDFium is not thread-safe, even across different documents. All calls must go through one thread or one lock per process. [1]
- The PDFium license is BSD-3-Clause, with the Apache-2.0 text also in the LICENSE file. The PRD says Apache 2.0. The non-V8 Windows DLL is 7.5 MB (3.9 MB compressed); the V8/XFA DLL is 32.2 MB (12.8 MB compressed). [2] [4]

## Findings

Proof setup: Python 3.14.5 venv in the scratch folder, pypdfium2 5.14.0 (raw C API through ctypes; bundles PDFium 156.0.8076.0 from pdfium-binaries) [10], pikepdf 10.16.0 (libqpdf 12.4.2) [11]. Headers and DLL exports were read from pdfium-binaries chromium/8086 (PDFium 157.0.8086.0) [4]. Test PDFs come from `make_test_pdfs.py`; they stay in the scratch folder. All timings are from the dev PC (i7-11800H, 32 GB, Windows 10, warm file cache). They are non-reference numbers.

```
$ python make_test_pdfs.py <scratch>/pdfs
big500.pdf                     52,069,951 bytes
form.pdf                            2,054 bytes
order.pdf                             791 bytes
password_aes256.pdf                 3,386 bytes
xfa_foreground.pdf                    763 bytes
xfa_full.pdf                          784 bytes
```

### 1. Coverage

| v1 task | PDFium API (header: function) | Status | Evidence |
| --- | --- | --- | --- |
| Render page | `fpdfview.h`: `FPDF_LoadPage`, `FPDF_RenderPageBitmap`, `FPDFBitmap_Create` | Supported | 773x1000 render, 165,233 non-white pixels (proof: coverage_probe.py) |
| Render tile | `fpdfview.h`: `FPDF_RenderPageBitmapWithMatrix` (matrix plus clip rect) | Supported | 512x512 tile at 4x (proof: coverage_probe.py) |
| Progressive render | `fpdf_progressive.h`: `FPDF_RenderPageBitmap_Start`, `FPDF_RenderPage_Continue`, `FPDF_RenderPage_Close`, `IFSDK_PAUSE` | Supported | Finished in 1 continue on a simple page; pause granularity on heavy pages not measured (proof: coverage_probe.py) |
| Form widgets in render | `fpdf_formfill.h`: `FPDF_FFLDraw` after the page render | Supported | `FPDF_ANNOT` skips widget and popup annotations [1] |
| Select and copy text | `fpdf_text.h`: `FPDFText_LoadPage`, `FPDFText_GetText`, `FPDFText_GetBoundedText`, `FPDFText_GetCharIndexAtPos`, `FPDFText_CountRects`, `FPDFText_GetRect`, `FPDFText_GetCharBox` | Partial | Text order is content-stream order, not visual order (proof: coverage_probe.py) |
| Text search | `fpdf_text.h`: `FPDFText_FindStart`, `FPDFText_FindNext`, `FPDFText_GetSchResultIndex`, `FPDFText_GetSchCount`, `FPDFText_FindClose`; flags `FPDF_MATCHCASE`, `FPDF_MATCHWHOLEWORD` | Supported | Case-insensitive by default (proof: coverage_probe.py) |
| AcroForm fill | `fpdf_formfill.h`: `FPDFDOC_InitFormFillEnvironment`, `FORM_OnAfterLoadPage`, `FORM_OnLButtonDown/Up`, `FORM_OnChar`, `FORM_OnKeyDown`, `FORM_ForceToKillFocus`, `FORM_GetFocusedAnnot`, `FORM_SetFocusedAnnot` | Supported | Values and widget `/AP` written on save (proof: coverage_probe.py) |
| Tab between fields | `FORM_OnKeyDown(FWL_VKEY_Tab)`, Shift with `FWL_EVENTFLAG_ShiftKey` | Partial | Works within a page. At the last field it stays put. Cross-page Tab is app code (proof: coverage_probe.py) |
| XFA detection | `fpdf_formfill.h`: `FPDF_GetFormType`; `fpdfview.h`: `FPDF_GetXFAPacketCount` | Supported | Returns 2 (XFA full) and 3 (XFA foreground) in the non-XFA build (proof: coverage_probe.py) |
| Highlight, underline, strikeout, squiggly | `fpdf_annot.h`: `FPDFPage_CreateAnnot`, `FPDFAnnot_AppendAttachmentPoints`, `FPDFAnnot_SetRect`, `FPDFAnnot_SetColor` | Supported | Created (proof: coverage_probe.py) |
| Sticky note | `FPDFPage_CreateAnnot(FPDF_ANNOT_TEXT)`, `FPDFAnnot_SetStringValue("Contents")` | Partial | Text and Popup annotations are creatable. No API sets `/Popup` or `/Parent`, so they cannot be linked [1] |
| Square, circle | `FPDFPage_CreateAnnot`, `FPDFAnnot_SetBorder` | Supported | Created (proof: coverage_probe.py) |
| Free text | `FPDFPage_CreateAnnot(FPDF_ANNOT_FREETEXT)`, `FPDFAnnot_SetStringValue("DA")` | Supported | Created; `/AP` generated after a render (proof: coverage_probe.py) |
| Ink (freehand, drawn signature) | `FPDFAnnot_AddInkStroke` | Supported | Created (proof: coverage_probe.py) |
| Line with arrow | `FPDFPage_CreateAnnot(FPDF_ANNOT_LINE)` | Not supported | Returns NULL. Workaround: Stamp annotation with a path object (`FPDFAnnot_AppendObject`) or an Ink annotation (proof: coverage_probe.py) |
| Polygon, polyline, caret, redact | `FPDFPage_CreateAnnot` | Not supported | Returns NULL (proof: coverage_probe.py) |
| Appearance streams | `FPDFAnnot_SetAP`, `FPDFAnnot_GetAP`; auto-generation at render | Partial | Not written unless the page renders with `FPDF_ANNOT` before save, or the app calls `FPDFAnnot_SetAP`. Popup never gets one. Stamp gets one at once (proof: coverage_probe.py) |
| Signature as stamp image | `FPDFPageObj_NewImageObj`, `FPDFImageObj_SetBitmap`, `FPDFImageObj_SetMatrix`, `FPDFAnnot_AppendObject` | Supported | Image XObject in `/AP`, alpha kept as `/SMask`, Flate-compressed (proof: coverage_probe.py) |
| Insert blank page | `fpdf_edit.h`: `FPDFPage_New` | Supported | (proof: coverage_probe.py) |
| Insert image as page | `FPDFPage_New`, `FPDFImageObj_LoadJpegFileInline`, `FPDFPage_InsertObject`, `FPDFPage_GenerateContent` | Supported | JPEG bytes pass through as `/DCTDecode`, 304,154 bytes in and out (proof: coverage_probe.py) |
| Delete page | `FPDFPage_Delete` | Supported | (proof: coverage_probe.py, incremental_save.py) |
| Reorder | `FPDF_MovePages` | Supported | (proof: coverage_probe.py, incremental_save.py) |
| Rotate | `FPDFPage_SetRotation` | Supported | `/Rotate 90` (proof: coverage_probe.py) |
| Crop | `fpdf_transformpage.h`: `FPDFPage_SetCropBox` | Supported | `/CropBox [50 50 550 400]` (proof: coverage_probe.py) |
| Merge | `fpdf_ppo.h`: `FPDF_ImportPages`, `FPDF_ImportPagesByIndex` | Partial | Pages and widgets copy. The destination gets no `/AcroForm`, so `FPDF_GetFormType` returns 0 and merged fields stop being fields (proof: coverage_probe.py) |
| Extract to new PDF | `FPDF_CreateNewDocument`, `FPDF_ImportPagesByIndex`, `FPDF_SaveAsCopy` | Supported | 2 of 500 pages saved as 208,806 bytes (proof: coverage_probe.py) |
| Incremental save | `fpdf_save.h`: `FPDF_SaveAsCopy(FPDF_INCREMENTAL)` | Partial | Appends all loaded objects, not only changed ones; see section 2 [3] |

Proof output:

```
$ python coverage_probe.py <scratch>/pdfs <scratch>/out
[render] FPDF_RenderPageBitmap 773x1000: 165233 non-white pixels
[render] FPDF_RenderPageBitmapWithMatrix 512x512 tile at 4x: 65241 non-white pixels
[render] progressive: FPDF_RenderPageBitmap_Start + 1 x FPDF_RenderPage_Continue, final status 2 (2=DONE), NeedToPauseNow called 1 times, 165233 non-white pixels
[text] FPDFText_GetText order: R1 right top | R2 right middle | L3 left bottom R3 right bottom | L2 left middle | L1 left top
[text] FPDFText_GetBoundedText(left column rect): 'L3 left bottom \r\nL2 left middle\r\nL1 left top'
[text] FPDFText_GetCharIndexAtPos(55,704) = 79; FPDFText_CountRects(idx, 11) = 1
[search] FPDFText_FindStart/FindNext page 500 'ZebraCrossing' (ignore case): found=True char index=1655 count=13
[search] same with FPDF_MATCHCASE: found=False
[forms] FPDF_GetFormType(big500.pdf) = 0  (0 none, 1 AcroForm, 2 XFA full, 3 XFA foreground)
[forms] FPDF_GetFormType(form.pdf) = 1  (0 none, 1 AcroForm, 2 XFA full, 3 XFA foreground)
[forms] FPDF_GetFormType(xfa_foreground.pdf) = 3  (0 none, 1 AcroForm, 2 XFA full, 3 XFA foreground)
[forms] FPDF_GetFormType(xfa_full.pdf) = 2  (0 none, 1 AcroForm, 2 XFA full, 3 XFA foreground)
[forms] click name, type, then Tab x3, Shift+Tab: page0:name -> page0:email -> page0:city -> page0:city -> shift+tab->page0:email
[forms] FORM_SetFocusedAnnot(notes) + type + Tab -> page1:agree; space pressed on it
[forms] after incremental save (field, /V, has /AP): [('name', 'Ada Lovelace', True), ('email', 'x', True), ('city', 'xx', True), ('notes', 'page two', True), ('agree', '/Yes', True)]
[annot] FPDFAnnot_IsSupportedSubtype: text(sticky)=True, popup=True, highlight=True, underline=True, strikeout=True, squiggly=True, line=False, square=True, circle=True, freetext=True, ink=True, stamp=True, polygon=False, polyline=False, caret=False, redact=False
[annot] FPDFPage_CreateAnnot result: text(sticky)=created, popup=created, highlight=created, underline=created, strikeout=created, squiggly=created, line=NULL, square=created, circle=created, freetext=created, ink=created, stamp:AppendObject(path)=created, stamp=created, polygon=NULL, polyline=NULL, caret=NULL, redact=NULL
[annot] /AP /N written (save_without_render): {'text(sticky)': False, 'popup': False, 'highlight': False, 'underline': False, 'strikeout': False, 'squiggly': False, 'square': False, 'circle': False, 'freetext': False, 'ink': False, 'stamp': True}
[annot] /AP /N written (render_then_save): {'text(sticky)': True, 'popup': False, 'highlight': True, 'underline': True, 'strikeout': True, 'squiggly': True, 'square': True, 'circle': True, 'freetext': True, 'ink': True, 'stamp': True}
[annot] /AP /N written (reload_render_save): {'text(sticky)': True, 'popup': False, 'highlight': True, 'underline': True, 'strikeout': True, 'squiggly': True, 'square': True, 'circle': True, 'freetext': True, 'ink': True, 'stamp': True}
[annot] FPDFAnnot_SetAP ok=1: /AP/N keys=['/BBox', '/Filter', '/Length', '/Subtype', '/Type'] BBox=[300, 370, 500, 400] stream=b'0 0 1 RG 1 w 300 370 200 30 re S'
[sign] stamp+image: SetBitmap=1 AppendObject=1 /AP/N XObject /Subtype=/Image /SMask=present 200x50
[pages] New/LoadJpegFileInline=1/GenerateContent=1/MovePages=1/ImportPages=1; pages=4; page1 /Rotate=90 /CropBox=[50, 50, 550, 400] image filter=/DCTDecode raw bytes=304154 (jpeg file 304154)
[pages] merge: widgets on merged pages=5; dest /AcroForm present=False
[pages] extract pages 1 and 500 to new PDF: FPDF_ImportPagesByIndex=1, 208,806 bytes
```

A separate check on the merged file printed `merged FPDF_GetFormType 0`.

Notes on the table:

- The annotation creation list in `fpdf_annot.h` matches the proof: circle, fileattachment, freetext, highlight, ink, link, popup, square, squiggly, stamp, strikeout, text, underline [1].
- `FPDFAnnot_SetColor` fails once an annotation has an appearance stream [1]. Set all properties first, then generate the `/AP`.
- `FPDFAnnot_SetAP` writes `/BBox` equal to the annotation `/Rect` and no `/Matrix`. The stream must use page coordinates (proof: coverage_probe.py).
- Annotations created by the API are written as direct dictionaries inside the page `/Annots` array, without `/F` or `/P` (proof: incremental_save.py appended section).

### 2. Incremental save

Source: `big500.pdf`, 52,069,951 bytes, classic xref table. Each case loads the file, applies the edit, and saves with `FPDF_SaveAsCopy`. "prefix_same" means the first 52,069,951 bytes of the output equal the source.

```
$ python incremental_save.py <scratch>/pdfs/big500.pdf <scratch>/out
source: 52,069,951 bytes, sha256 7a62548210f3ab95
case                                        save ms    out bytes       added prefix_same qpdf_pages pdfium_pages annots_p0 /AP
A_highlight_incremental                       104.3   52,179,039     109,088 True               500          500         1 False
B_render_all_then_highlight_incremental       239.2  104,130,031  52,060,080 True               500          500         1 False
C_move_last_to_first_incremental              141.1   52,160,072      90,121 True               500          500         0 False
D_delete_page_10_incremental                  116.6   52,159,940      89,989 True               499          499         0 False
E_highlight_full_rewrite                      133.9   52,060,082      -9,869 False              500          500         1 False
```

(The script also prints the first 600 bytes of case A's appended section; omitted here.)

Object analysis of the appended sections:

| Case | Objects appended | What they are |
| --- | --- | --- |
| A highlight | 6 | Catalog, page tree (4,447 bytes), page 1 dict with the annotation, its content stream (1,024 bytes), font, image (102,881 bytes). Xref section: 310 bytes. Trailer has `/Prev 52039722`. |
| B render all, then highlight | 1,503 | 500 content streams, 499 images, page dicts, page tree, catalog |
| C move last page to first | 502 | Catalog, page tree, 500 page dicts |
| D delete page 10 | 502 | Same as C |

Findings:

- Incremental save works after a highlight, a page move, and a page delete. The original bytes stay unchanged. qpdf and PDFium both open every output. (proof: incremental_save.py)
- PDFium writes every object present in the document's in-memory object map, changed or not. `CPDF_Creator::InitNewObjNumOffsets` iterates the whole map and, in incremental mode, does not skip objects that already exist in the file [3]. So the appended size depends on what the session loaded, not on what changed.
- `FPDF_LoadPage` parses the page content and resources. Editing a page therefore re-appends that page's images and fonts. For a scanned page with a 2 MB image, one highlight appends about 2 MB (unverified; extrapolated from case A).
- Moving or deleting pages loads all page dicts: about 90 KB for 500 pages.
- A full rewrite of 52 MB took 134 ms on this PC, because PDFium copies unchanged streams without decoding.

### 3. Password PDFs

Open with `FPDF_LoadDocument(path, password)` or `FPDF_LoadMemDocument64`. On failure, `FPDF_GetLastError` returns one of `FPDF_ERR_SUCCESS 0`, `UNKNOWN 1`, `FILE 2`, `FORMAT 3`, `PASSWORD 4`, `SECURITY 5` (unsupported security scheme), `PAGE 6` [1].

```
$ python password_open.py <scratch>/pdfs/form.pdf <scratch>/out
== R6_AES256
  open with no password    : FAIL FPDF_GetLastError=4 (PASSWORD)
  open with wrong password : FAIL FPDF_GetLastError=4 (PASSWORD)
  open with user password  : OK FPDF_GetLastError=0 (SUCCESS) rev=6 perms=0xFFFFFBFC user_perms=0xFFFFFBFC
  open with owner password : OK FPDF_GetLastError=0 (SUCCESS) rev=6 perms=0xFFFFFFFC user_perms=0xFFFFFBFC
  save INCREMENTAL    : appended=True -> no-pw:PASSWORD user-pw:opens rev:6
  save NO_INCREMENTAL : appended=False -> no-pw:PASSWORD user-pw:opens rev:6
  save REMOVE_SECURITY: appended=False -> no-pw:opens user-pw:opens rev:-1
== R4_AES128
  open with no password    : FAIL FPDF_GetLastError=4 (PASSWORD)
  open with wrong password : FAIL FPDF_GetLastError=4 (PASSWORD)
  open with user password  : OK FPDF_GetLastError=0 (SUCCESS) rev=4 perms=0xFFFFFBFC user_perms=0xFFFFFBFC
  open with owner password : OK FPDF_GetLastError=0 (SUCCESS) rev=4 perms=0xFFFFFFFC user_perms=0xFFFFFBFC
  save INCREMENTAL    : appended=True -> no-pw:PASSWORD user-pw:opens rev:4
  save NO_INCREMENTAL : appended=False -> no-pw:PASSWORD user-pw:opens rev:4
  save REMOVE_SECURITY: appended=False -> no-pw:opens user-pw:opens rev:-1
== R3_RC4_128
  open with no password    : FAIL FPDF_GetLastError=4 (PASSWORD)
  open with wrong password : FAIL FPDF_GetLastError=4 (PASSWORD)
  open with user password  : OK FPDF_GetLastError=0 (SUCCESS) rev=3 perms=0xFFFFFBFC user_perms=0xFFFFFBFC
  open with owner password : OK FPDF_GetLastError=0 (SUCCESS) rev=3 perms=0xFFFFFFFC user_perms=0xFFFFFBFC
  save INCREMENTAL    : appended=True -> no-pw:PASSWORD user-pw:opens rev:3
  save NO_INCREMENTAL : appended=False -> no-pw:PASSWORD user-pw:opens rev:3
  save REMOVE_SECURITY: appended=False -> no-pw:opens user-pw:opens rev:-1
== R6_owner_only
  open with no password    : OK FPDF_GetLastError=0 (SUCCESS) rev=6 perms=0xFFFFFBFC user_perms=0xFFFFFBFC
  open with wrong password : FAIL FPDF_GetLastError=4 (PASSWORD)
  open with user password  : FAIL FPDF_GetLastError=4 (PASSWORD)
  open with owner password : OK FPDF_GetLastError=0 (SUCCESS) rev=6 perms=0xFFFFFFFC user_perms=0xFFFFFBFC
  save INCREMENTAL    : appended=True -> no-pw:opens user-pw:PASSWORD rev:6
  save NO_INCREMENTAL : appended=False -> no-pw:opens user-pw:PASSWORD rev:6
  save REMOVE_SECURITY: appended=False -> no-pw:opens user-pw:opens rev:-1
```

Findings:

- "No password" and "wrong password" both return error 4. The app tells them apart by whether it passed a password. (proof: password_open.py)
- The owner password unlocks the permission bits that the user password restricts (0xFFFFFFFC against 0xFFFFFBFC). (proof: password_open.py)
- PDFium keeps existing RC4-128, AES-128, and AES-256 encryption on incremental and full saves. `FPDF_REMOVE_SECURITY` writes a decrypted file. (proof: password_open.py)
- Error 5 (unsupported scheme, for example certificate encryption) was not tested (unverified).

### 4. Encryption gap

Confirmed: PDFium cannot add or change encryption. `fpdf_save.h` has only `FPDF_INCREMENTAL`, `FPDF_NO_INCREMENTAL`, `FPDF_REMOVE_SECURITY`, and `FPDF_SUBSET_NEW_FONTS` [1]. The DLL exports only read-only security getters:

```
$ python encrypt_gap.py <pdfium.dll chromium/8086 win-x64> <qpdf-12.4.2-msvc64/bin> <scratch>/out/A_highlight_incremental.pdf <scratch>/out
pdfium.dll exports: 471; names containing encrypt/password/secur/permission: ['FPDFSignatureObj_GetDocMDPPermission', 'FPDF_GetDocPermissions', 'FPDF_GetDocUserPermissions', 'FPDF_GetSecurityHandlerRevision']
qpdf CLI exit 0 in 1496 ms; PDFium: no password -> err 4; user password -> opens, revision 6, pages 500
qpdf C API 12.4.2: return codes [0, 0, 0] in 1208 ms; PDFium: no password -> err 4; user password -> opens, revision 6, pages 500
```

How qpdf fills the gap:

- CLI: `qpdf --encrypt --user-password=U --owner-password=O --bits=256 -- in.pdf out.pdf`. 40-bit and RC4 128-bit need `--allow-weak-crypto` (from `qpdf --help=encryption`, qpdf 12.4.2).
- C API (`qpdf-c.h`): `qpdf_init`, `qpdf_read`, `qpdf_init_write`, `qpdf_set_r6_encryption_parameters2`, `qpdf_write`, `qpdf_cleanup` [9]. Proved by `encrypt_gap.py` through ctypes on the official `qpdf30.dll`.
- License: Apache-2.0 since qpdf 7; earlier versions were Artistic-2.0 [6] [7].
- Release: 12.4.2, 2026-09-27, with `msvc64` and, new in this release, `msvc-arm64` zips. SHA-256 sums matched the release `.sha256` file [5].
- Crypto providers in the official Windows x64 build: `qpdf --show-crypto` prints `openssl` (default) and `native`. The Windows build links zlib, jpeg, and OpenSSL from qpdf's vcpkg cache [8]. The native provider uses public-domain Rijndael code and MIT-style sha2 code from sphlib [7]. GnuTLS is a third provider option, not in the Windows build [6]. OpenSSL version inside `qpdf30.dll` was not found (unverified). OpenSSL 3.x license is Apache-2.0 (unverified; not fetched).
- Size: `qpdf30.dll` is 7,448,064 bytes (x64) and 6,978,048 bytes (arm64). It needs the MSVC runtime DLLs (`msvcp140.dll`, `vcruntime140.dll`, `vcruntime140_1.dll`, `concrt140.dll`, and others, about 1.6 MB on x64), which ship in the zip.
- qpdf rewrites the whole file. It has no incremental writer (unverified).

### 5. Thread safety

`fpdfview.h` states: "None of the PDFium APIs are thread-safe. They expect to be called from a single thread." Embedders must make sure only one PDFium call runs at a time [1]. This applies across documents too, because the header makes no per-document exception.

Implications:

- One PDFium worker thread per process, or one global lock. Page render, thumbnails, text extraction, and saves share it.
- "Thumbnails on a background thread" (PRD) works only if that thread is the PDFium thread, with page render requests at higher priority.
- True parallel rendering needs a second process with its own PDFium instance (unverified cost).
- Measured single-thread cost is small: 2.0 ms per 150x194 thumbnail and 1.1 ms per 512x512 tile on the synthetic PDF (section 8).

### 6. Binaries

Best source: bblanchon/pdfium-binaries [4]. It also publishes NuGet packages; `bblanchon.PDFium.Win32` 157.0.8086 contains `runtimes/win-x64`, `win-arm64`, and `win-x86` `pdfium.dll` [12].

| Item | Value |
| --- | --- |
| Latest release | chromium/8086 = PDFium 157.0.8086.0, 2026-10-05 [4] |
| Cadence | 12 releases between 2026-07-13 and 2026-10-05, about 1 per week [4] |
| Build flags (non-V8) | `pdf_enable_v8 = false`, `pdf_enable_xfa = false`, `is_debug = false` (`args.gn` in the tgz) |
| Build flags (V8) | `pdf_enable_v8 = true`, `pdf_enable_xfa = true`, `v8_enable_i18n_support = false` |
| win-x64 `pdfium.dll` | 7,494,656 bytes; tgz 3.87 MB |
| win-arm64 `pdfium.dll` | 6,871,552 bytes; tgz 3.64 MB |
| v8-win-x64 `pdfium.dll` | 32,187,904 bytes; tgz 12.82 MB |
| v8-win-arm64 `pdfium.dll` | 29,894,656 bytes; tgz 11.94 MB |
| Packaging license | MIT, Benoit Blanchon (`LICENSE` in the tgz). NuGet lists Apache-2.0 [12]. The two disagree. |
| License files (non-V8) | 18 files in `licenses/`: abseil, agg23, dragonbox (Apache2-LLVM and Boost), fast_float, freetype, harfbuzz, icu, lcms, libjpeg_turbo (`.ijg` and `.md`), libopenjpeg, libpng, llvm-libc, pdfium, simdutf, zlib |
| Extra license files (V8) | bigint, fdlibm, fp16, highway, inspector_protocol, libtiff, strongtalk, v8 |
| Provenance | Each release ships `pdfium-attestation.json` [4] |

Installer impact against the 30 MB target, per architecture: non-V8 PDFium costs about 3.9 MB compressed. V8/XFA costs about 12.8 MB, 43% of the budget. qpdf adds about 9 MB uncompressed (DLL plus runtime); its compressed size was not measured (unverified).

### 7. C# interop for NativeAOT

| Option | Version, date | License | Maintenance | AOT | ARM64 | Fit |
| --- | --- | --- | --- | --- | --- | --- |
| Hand-written `[LibraryImport]` | n/a | ours | ours | Designed for it: source-generated marshalling (unverified; no build proof) | Yes, via bblanchon binaries | Best |
| ClangSharpPInvokeGenerator output | not tested | MIT (unverified) | n/a | Emits blittable `DllImport` (unverified) | n/a | Useful once to draft signatures |
| PDFiumCore | 156.0.8076, 2026-10-01 [13] | Apache-2.0 | Tracks PDFium builds | Unverified | Depends on `bblanchon.PDFium.Win32`, which has win-arm64 [12] | Possible fallback |
| PDFtoImage | 5.4.0, 2026-08-16 [14] | MIT | Active | Unverified | Unverified | No: render-to-image only, pulls in SkiaSharp |
| Docnet.Core | 2.6.0, 2023-09-04 [15] | MIT | Stale | Unverified | No win-arm64 DLL in the package | No |
| PdfiumViewer | 2.13.0, 2017-11-06 [16] | Apache-2.0 | Dead | No | No | No |
| PDFiumSharpV2 | 1.1.4, 2024-01-10 [17] | MS-RL (reciprocal) | Stale | Unverified | Unverified | No: license |
| Pdfium.Net.SDK | 5.1.4, 2026-10-05 [18] | Commercial | Active | Unverified | Unverified | No: paid |

Recommendation: hand-write `[LibraryImport]` declarations for the about 120 functions v1 needs. Use `[UnmanagedCallersOnly]` function pointers for the 3 callback structs (`FPDF_FILEWRITE`, `FPDF_FILEACCESS`, `IFSDK_PAUSE`) and for `FPDF_FORMFILLINFO`. Take binaries from `bblanchon.PDFium.Win32`. The NativeAOT build proof was dropped for budget (unverified).

### 8. Performance (non-reference)

PDFium 156.0.8076.0 through pypdfium2 5.14.0. The PDF is synthetic: Helvetica text (about 3,600 characters per page) plus one 185x185 RGB image per page, classic xref. Real PDFs with embedded fonts, vector art, or a broken xref will be slower.

```
$ python perf.py <scratch>/pdfs/big500.pdf
first page 100% scale 773x1000: open+count median     2.0 ms  min     1.6  max     6.7  (n=9)
                                LoadPage   median     1.8 ms  min     1.2  max     3.4  (n=9)
                                render     median     5.1 ms  min     4.2  max     5.9  (n=9)
                                TOTAL      median    10.0 ms  min     8.3  max    13.2  (n=9)
first page 150% scale 1160x1500: open+count median     2.3 ms  min     1.8  max     4.2  (n=9)
                                LoadPage   median     1.8 ms  min     1.4  max     2.1  (n=9)
                                render     median     7.8 ms  min     6.9  max     8.8  (n=9)
                                TOTAL      median    11.9 ms  min    10.7  max    14.5  (n=9)
tile 512x512 at 200% with content (RenderPageBitmapWithMatrix): median     1.1 ms  min     0.4  max     1.6  (n=10)
thumbnails 150x194, 500 pages, one thread: total 1091 ms, per page median     2.0 ms  min     1.4  max     9.4  (n=500)
search 'zebracrossing' over 500 pages (fresh doc, LoadPage+LoadTextPage+Find per page): median   857.9 ms  min   811.2  max   951.3  (n=3), hits 1
search 'invoice' over 500 pages (fresh doc, LoadPage+LoadTextPage+Find per page): median   868.1 ms  min   862.6  max   990.1  (n=3), hits 10362
extract all text once: 820 ms (1,816,501 chars); in-memory search 'invoice': 3.0 ms, hits 10362
```

Reading: open plus first page takes 10 to 12 ms, far below the 300 ms target. A cold search through PDFium takes 0.86 s on this faster PC, close to the 1 s target. Text extracted once and searched in memory takes 3 ms.

### 9. Known PDFium gaps that hit v1

- XFA without V8: the non-V8 build has `pdf_enable_xfa = false` [4]. XFA-full forms show only their fallback page (unverified for real forms). Detection works: `FPDF_GetFormType` returns 2 or 3 (proof: coverage_probe.py).
- Form JavaScript: the non-V8 build has `pdf_enable_v8 = false`, so calculate, format, and validate scripts do not run [4]. `fpdf_javascript.h` can list document scripts (`FPDFDoc_GetJavaScriptActionCount`) to warn the user [1].
- Digital signatures: `fpdf_signature.h` only reads signatures (`FPDF_GetSignatureCount`, `FPDFSignatureObj_GetContents`, `GetByteRange`, `GetSubFilter`, `GetReason`, `GetTime`, `GetDocMDPPermission`). PDFium cannot create or verify a digital signature [1]. v1 signatures are ink or stamp images, which PDFium supports.
- Signed PDFs: incremental save keeps the signed byte range intact; a full rewrite breaks the signature (unverified; follows from section 2).
- Annotation types it cannot create: Line, Polygon, Polyline, Caret, Redact (proof: coverage_probe.py).
- Popup linking: no setter for `/Popup` or `/Parent` [1].
- Merge drops `/AcroForm` (proof: coverage_probe.py).
- Bug tracker entries for these gaps were not collected (unverified).

## Confidence

| Finding | Confidence | Reason |
| --- | --- | --- |
| API coverage table | High | Each row ran against PDFium 156.0.8076; headers checked at 157.0.8086 |
| Incremental save appends all loaded objects | High | Measured and matched to `cpdf_creator.cpp` at chromium/8086 [3] |
| Missing `/AP` on API-created annotations | High | Measured on 11 subtypes |
| Acrobat and Edge display of these annotations | Low | Not tested here |
| PDFium cannot encrypt; keeps existing encryption | High | Export table plus round-trip proof |
| qpdf encrypts via CLI and C API | High | Proof with official 12.4.2 binaries |
| Not thread-safe | High | Primary source header [1] |
| Binary sizes and licenses | High | Read from the release tgz files [4] |
| C# wrapper ranking | Medium | NuGet metadata only; no AOT build |
| Performance numbers | Low for PRD gates | Synthetic PDF, faster PC, warm cache |

## Conflicts with the PRD

1. License. The PRD lists PDFium as Apache 2.0. The PDFium LICENSE file holds BSD-3-Clause text followed by Apache-2.0 text, and the headers say "BSD-style license" [2]. Both are permissive. The BSD binary clause and 17 bundled third-party licenses (for example FreeType's credit clause) require a notices file in the app [4].
2. "Save edits incrementally; never rewrite a 50 MB PDF to add one highlight." `FPDF_INCREMENTAL` alone breaks this: after scrolling all pages, one highlight appended 52 MB (proof: incremental_save.py). It holds only with a separate save document (recommendation 3).
3. Task 5, "saved as standard PDF annotations readable in Acrobat and Edge." PDFium cannot create Line annotations, so arrows become Stamp or Ink annotations. PDFium does not write `/AP` unless the app forces it (proof: coverage_probe.py).
4. Task 6, "drop a file into the sidebar merges it." Merged pages lose form fields (proof: coverage_probe.py).
5. Task 2, "copy keeps reading order." PDFium returns content-stream order (proof: coverage_probe.py).
6. Task 2, "Ctrl+F finds text in a 500-page PDF in under 1 s." A cold PDFium search took 0.86 s on this faster PC with simple pages. The reference laptop and real files put it at risk (proof: perf.py).
7. "Thumbnails on a background thread." PDFium allows one call at a time per process [1].
8. Stack table puts qpdf in v1. Passwords are v2, and PDFium keeps existing encryption on save. qpdf is not needed in v1 unless it fixes form merges.

## What we should do

1. Ship the non-V8 pdfium-binaries build for win-x64 and win-arm64. Pin one release; update monthly; check `pdfium-attestation.json`.
2. Run all PDFium calls on one dedicated thread with a priority queue: visible page render first, then tiles, thumbnails, text extraction.
3. Save from a second, fresh `FPDF_DOCUMENT`: keep an edit list, open the file again, load only touched pages, replay the edits, call `FPDF_SaveAsCopy(FPDF_INCREMENTAL)` to a temp file, then replace. Measured: +109 KB in 104 ms for one highlight.
4. Always produce `/AP`: set every property, then render the page once with `FPDF_ANNOT` before save, or call `FPDFAnnot_SetAP`. Add Acrobat and Edge round-trip tests in the week-3 spike.
5. Draw arrows as Stamp annotations with path objects (`FPDFAnnot_AppendObject`); `/AP` is generated at once.
6. Implement cross-page Tab in the app: when focus does not change, call `FORM_SetFocusedAnnot` on the first widget of the next page. Draw widgets with `FPDF_FFLDraw`.
7. Extract each page's text once in the background after open, cache it, and search in memory (3 ms for 1.8 M characters).
8. Keep content-stream order for copy in v1. Add a line and column sort using `FPDFText_GetCharBox` only if usability tests show a problem.
9. For merges where the source has `/AcroForm`, spike qpdf's form-aware page merge before v1 (unverified that it fixes this).
10. Add qpdf in v2 for passwords, through its C API. Budget about 9 MB uncompressed per architecture.
11. Write C# bindings by hand with `[LibraryImport]`; prove NativeAOT on x64 and ARM64 in the week-1 spike.
12. Ship a THIRD-PARTY-NOTICES file built from the `licenses/` folder.

## Open questions

- NativeAOT `[LibraryImport]` proof (open PDF, render page 1, callbacks) was not built.
- ClangSharpPInvokeGenerator output quality and PDFiumCore AOT compatibility are untested.
- Rust bindings (for example the pdfium-render crate) were not researched.
- Do Acrobat Reader and Edge display PDFium-generated `/AP`, Stamp-based arrows, and Text annotations without a linked Popup?
- Does qpdf's page merge keep AcroForm fields, and at what size cost?
- XFA rendering with the V8 build was not tested with a real XFA form.
- Progressive render pause granularity on heavy pages is unmeasured.
- Does `FPDF_LoadDocument` hold a file lock that blocks replace-on-save while the view document is open?
- `FPDF_ERR_SECURITY` behavior on certificate-encrypted PDFs.
- OpenSSL version and license notice inside `qpdf30.dll`; whether qpdf has any incremental writer.
- PDFium bug tracker entries for each gap in section 9.
- Performance on the reference laptop with a real 500-page corpus.

## Sources

1. PDFium public headers, chromium/8086 (`fpdfview.h`, `fpdf_save.h`, `fpdf_annot.h`, `fpdf_formfill.h`, `fpdf_edit.h`, `fpdf_ppo.h`, `fpdf_text.h`, `fpdf_progressive.h`, `fpdf_transformpage.h`, `fpdf_signature.h`, `fpdf_javascript.h`), https://pdfium.googlesource.com/pdfium/+/refs/heads/chromium/8086/public/
2. PDFium LICENSE, https://pdfium.googlesource.com/pdfium/+/refs/heads/main/LICENSE
3. PDFium `cpdf_creator.cpp`, chromium/8086, https://pdfium.googlesource.com/pdfium/+/refs/heads/chromium/8086/core/fpdfapi/edit/cpdf_creator.cpp
4. bblanchon/pdfium-binaries releases (chromium/8086 and earlier), https://github.com/bblanchon/pdfium-binaries/releases/tag/chromium/8086
5. qpdf v12.4.2 release, https://github.com/qpdf/qpdf/releases/tag/v12.4.2
6. qpdf README (license, crypto providers), https://github.com/qpdf/qpdf/blob/v12.4.2/README.md
7. qpdf NOTICE, https://github.com/qpdf/qpdf/blob/v12.4.2/NOTICE.md
8. qpdf README-windows, https://github.com/qpdf/qpdf/blob/v12.4.2/README-windows.md
9. qpdf C API header `qpdf-c.h`, https://github.com/qpdf/qpdf/blob/v12.4.2/include/qpdf/qpdf-c.h
10. pypdfium2, https://pypi.org/project/pypdfium2/
11. pikepdf, https://pypi.org/project/pikepdf/
12. bblanchon.PDFium.Win32 NuGet package, https://www.nuget.org/packages/bblanchon.PDFium.Win32
13. PDFiumCore NuGet package, https://www.nuget.org/packages/PDFiumCore
14. PDFtoImage NuGet package, https://www.nuget.org/packages/PDFtoImage
15. Docnet.Core NuGet package, https://www.nuget.org/packages/Docnet.Core
16. PdfiumViewer NuGet package, https://www.nuget.org/packages/PdfiumViewer
17. PDFiumSharpV2 NuGet package, https://www.nuget.org/packages/PDFiumSharpV2
18. Pdfium.Net.SDK NuGet package, https://www.nuget.org/packages/Pdfium.Net.SDK
