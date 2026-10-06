# Proves which v1 PDF tasks the PDFium C API covers: calls each API once and prints the observed result.
# Usage: python coverage_probe.py <pdf_dir from make_test_pdfs.py> <out_dir>
import ctypes, os, sys
import pikepdf
import pypdfium2_raw as r
from pdfium_common import init, load, save_bytes, page_text, wide

D, OUT = sys.argv[1], sys.argv[2]
P = lambda n: os.path.join(D, n)
init()


def say(section, msg):
    print(f"[{section}] {msg}")


def nonwhite(bmp, w, h):
    stride = r.FPDFBitmap_GetStride(bmp)
    data = ctypes.string_at(r.FPDFBitmap_GetBuffer(bmp), stride * h)
    return sum(1 for i in range(0, len(data), 4) if data[i:i + 3] != b"\xff\xff\xff")


def white_bitmap(w, h):
    bmp = r.FPDFBitmap_Create(w, h, 0)
    r.FPDFBitmap_FillRect(bmp, 0, 0, w, h, 0xFFFFFFFF)
    return bmp


def write(name, data):
    path = os.path.join(OUT, name)
    open(path, "wb").write(data)
    return path


# ---------- 1. Render ----------
doc, _ = load(P("big500.pdf"))
page = r.FPDF_LoadPage(doc, 0)
W, H = 773, 1000  # fit-to-height on a 1080p screen
bmp = white_bitmap(W, H)
r.FPDF_RenderPageBitmap(bmp, page, 0, 0, W, H, 0, r.FPDF_ANNOT)
say("render", f"FPDF_RenderPageBitmap {W}x{H}: {nonwhite(bmp, W, H)} non-white pixels")
r.FPDFBitmap_Destroy(bmp)

tile = white_bitmap(512, 512)  # 512x512 tile of the page at 400% zoom, tile origin (1024, 1024)
m = r.FS_MATRIX(4, 0, 0, 4, -1024, -1024)
r.FPDF_RenderPageBitmapWithMatrix(tile, page, ctypes.byref(m), ctypes.byref(r.FS_RECTF(0, 0, 512, 512)), r.FPDF_ANNOT)
say("render", f"FPDF_RenderPageBitmapWithMatrix 512x512 tile at 4x: {nonwhite(tile, 512, 512)} non-white pixels")
r.FPDFBitmap_Destroy(tile)

calls = {"pause_checks": 0}
pause_fn = dict(r.IFSDK_PAUSE._fields_)["NeedToPauseNow"]


def need_pause(_):
    calls["pause_checks"] += 1
    return 1  # always ask to pause: worst case for a UI thread that wants to stay responsive


pause = r.IFSDK_PAUSE(version=1)
pause.NeedToPauseNow = pause_fn(need_pause)
bmp = white_bitmap(W, H)
status = r.FPDF_RenderPageBitmap_Start(bmp, page, 0, 0, W, H, 0, r.FPDF_ANNOT, ctypes.byref(pause))
steps = 1
while status == r.FPDF_RENDER_TOBECONTINUED:
    status = r.FPDF_RenderPage_Continue(page, ctypes.byref(pause))
    steps += 1
r.FPDF_RenderPage_Close(page)
say("render", f"progressive: FPDF_RenderPageBitmap_Start + {steps - 1} x FPDF_RenderPage_Continue, final status {status}"
              f" (2=DONE), NeedToPauseNow called {calls['pause_checks']} times, {nonwhite(bmp, W, H)} non-white pixels")
r.FPDFBitmap_Destroy(bmp)
r.FPDF_ClosePage(page)

# ---------- 2. Text select / copy, reading order ----------
odoc, _ = load(P("order.pdf"))
opage = r.FPDF_LoadPage(odoc, 0)
tp = r.FPDFText_LoadPage(opage)
say("text", "FPDFText_GetText order: " + " | ".join(page_text(tp).split("\r\n")))
buf = (ctypes.c_ushort * 200)()
n = r.FPDFText_GetBoundedText(tp, 40, 720, 300, 650, buf, 200)  # left column only
say("text", "FPDFText_GetBoundedText(left column rect): " + repr(bytes(buf)[: max(n - 1, 0) * 2].decode("utf-16-le")))
idx = r.FPDFText_GetCharIndexAtPos(tp, 55, 704, 5, 5)
say("text", f"FPDFText_GetCharIndexAtPos(55,704) = {idx}; FPDFText_CountRects(idx, 11) = {r.FPDFText_CountRects(tp, idx, 11)}")
r.FPDFText_ClosePage(tp)
r.FPDF_ClosePage(opage)
r.FPDF_CloseDocument(odoc)

# ---------- 3. Search ----------
page = r.FPDF_LoadPage(doc, 499)
tp = r.FPDFText_LoadPage(page)
h = r.FPDFText_FindStart(tp, wide("ZebraCrossing"), 0, 0)  # flags 0 = case-insensitive
found = r.FPDFText_FindNext(h)
say("search", f"FPDFText_FindStart/FindNext page 500 'ZebraCrossing' (ignore case): found={bool(found)} "
              f"char index={r.FPDFText_GetSchResultIndex(h)} count={r.FPDFText_GetSchCount(h)}")
r.FPDFText_FindClose(h)
h = r.FPDFText_FindStart(tp, wide("ZebraCrossing"), r.FPDF_MATCHCASE, 0)
say("search", f"same with FPDF_MATCHCASE: found={bool(r.FPDFText_FindNext(h))}")
r.FPDFText_FindClose(h)
r.FPDFText_ClosePage(tp)
r.FPDF_ClosePage(page)
r.FPDF_CloseDocument(doc)

# ---------- 4. Forms ----------
for n in ["big500.pdf", "form.pdf", "xfa_foreground.pdf", "xfa_full.pdf"]:
    d, _ = load(P(n))
    say("forms", f"FPDF_GetFormType({n}) = {r.FPDF_GetFormType(d)}  (0 none, 1 AcroForm, 2 XFA full, 3 XFA foreground)")
    r.FPDF_CloseDocument(d)

fdoc, _ = load(P("form.pdf"))
info = r.FPDF_FORMFILLINFO(version=2)
form = r.FPDFDOC_InitFormFillEnvironment(fdoc, ctypes.byref(info))
pages = [r.FPDF_LoadPage(fdoc, i) for i in range(2)]
for p in pages:
    r.FORM_OnAfterLoadPage(p, form)


def focused():
    pi, a = ctypes.c_int(-1), r.FPDF_ANNOTATION()
    r.FORM_GetFocusedAnnot(form, ctypes.byref(pi), ctypes.byref(a))
    if not a:
        return "none"
    b = (ctypes.c_ushort * 64)()
    k = r.FPDFAnnot_GetFormFieldName(form, a, b, 128)
    name = bytes(b)[: max(k - 2, 0)].decode("utf-16-le")
    r.FPDFPage_CloseAnnot(a)
    return f"page{pi.value}:{name}"


def type_text(p, s):
    for ch in s:
        r.FORM_OnChar(form, p, ord(ch), 0)


p0, p1 = pages
r.FORM_OnLButtonDown(form, p0, 0, 150, 712)  # click into "name"
r.FORM_OnLButtonUp(form, p0, 0, 150, 712)
trail = [focused()]
type_text(p0, "Ada Lovelace")
for _ in range(3):
    r.FORM_OnKeyDown(form, p0, r.FWL_VKEY_Tab, 0)
    trail.append(focused())
    type_text(p0, "x")
r.FORM_OnKeyDown(form, p0, r.FWL_VKEY_Tab, r.FWL_EVENTFLAG_ShiftKey)
trail.append("shift+tab->" + focused())
say("forms", "click name, type, then Tab x3, Shift+Tab: " + " -> ".join(trail))
annot = r.FPDFPage_GetAnnot(p1, 0)  # "notes" on page 2: focus by API (cross-page Tab is the app's job)
r.FORM_SetFocusedAnnot(form, annot)
r.FPDFPage_CloseAnnot(annot)
type_text(p1, "page two")
r.FORM_OnKeyDown(form, p1, r.FWL_VKEY_Tab, 0)
f2 = focused()
r.FORM_OnChar(form, p1, ord(" "), 0)  # space toggles a focused checkbox
say("forms", f"FORM_SetFocusedAnnot(notes) + type + Tab -> {f2}; space pressed on it")
r.FORM_ForceToKillFocus(form)
for p in pages:
    r.FORM_OnBeforeClosePage(p, form)
    r.FPDF_ClosePage(p)
r.FPDFDOC_ExitFormFillEnvironment(form)
path = write("form_filled.pdf", save_bytes(fdoc, r.FPDF_INCREMENTAL))
r.FPDF_CloseDocument(fdoc)
with pikepdf.open(path) as pk:
    vals = [(str(f.T), str(f.get("/V")), "/AP" in f) for f in pk.Root.AcroForm.Fields]
say("forms", f"after incremental save (field, /V, has /AP): {vals}")

# ---------- 5. Annotations ----------
SUB = {"text(sticky)": r.FPDF_ANNOT_TEXT, "popup": r.FPDF_ANNOT_POPUP, "highlight": r.FPDF_ANNOT_HIGHLIGHT,
       "underline": r.FPDF_ANNOT_UNDERLINE, "strikeout": r.FPDF_ANNOT_STRIKEOUT, "squiggly": r.FPDF_ANNOT_SQUIGGLY,
       "line": r.FPDF_ANNOT_LINE, "square": r.FPDF_ANNOT_SQUARE, "circle": r.FPDF_ANNOT_CIRCLE,
       "freetext": r.FPDF_ANNOT_FREETEXT, "ink": r.FPDF_ANNOT_INK, "stamp": r.FPDF_ANNOT_STAMP,
       "polygon": r.FPDF_ANNOT_POLYGON, "polyline": r.FPDF_ANNOT_POLYLINE, "caret": r.FPDF_ANNOT_CARET,
       "redact": r.FPDF_ANNOT_REDACT}


def make_annots(d, page):
    made = {}
    y = 740
    for name, st in SUB.items():
        a = r.FPDFPage_CreateAnnot(page, st)
        if not a:
            made[name] = None
            continue
        y -= 40
        r.FPDFAnnot_SetRect(a, ctypes.byref(r.FS_RECTF(60, y + 30, 260, y)))
        r.FPDFAnnot_SetColor(a, r.FPDFANNOT_COLORTYPE_Color, 200, 0, 0, 255)
        r.FPDFAnnot_SetStringValue(a, b"Contents", wide(name))
        r.FPDFAnnot_SetStringValue(a, b"T", wide("probe"))
        if r.FPDFAnnot_HasAttachmentPoints(a):
            r.FPDFAnnot_AppendAttachmentPoints(a, ctypes.byref(r.FS_QUADPOINTSF(60, y + 30, 260, y + 30, 60, y, 260, y)))
        if st == r.FPDF_ANNOT_INK:
            pts = (r.FS_POINTF * 3)(r.FS_POINTF(60, y), r.FS_POINTF(160, y + 30), r.FS_POINTF(260, y))
            r.FPDFAnnot_AddInkStroke(a, pts, 3)
        if st == r.FPDF_ANNOT_FREETEXT:
            r.FPDFAnnot_SetStringValue(a, b"DA", wide("/Helv 12 Tf 0 0 1 rg"))
        if st == r.FPDF_ANNOT_STAMP:  # arrow drawn as a path object inside a stamp
            path = r.FPDFPageObj_CreateNewPath(60, y + 15)
            r.FPDFPath_LineTo(path, 250, y + 15)
            r.FPDFPath_MoveTo(path, 240, y + 25)
            r.FPDFPath_LineTo(path, 250, y + 15)
            r.FPDFPath_LineTo(path, 240, y + 5)
            r.FPDFPageObj_SetStrokeColor(path, 0, 0, 255, 255)
            r.FPDFPageObj_SetStrokeWidth(path, 2)
            r.FPDFPath_SetDrawMode(path, 0, 1)
            made["stamp:AppendObject(path)"] = bool(r.FPDFAnnot_AppendObject(a, path))
        made[name] = True
        r.FPDFPage_CloseAnnot(a)
    return made


def ap_report(path):
    with pikepdf.open(path) as pk:
        out = {}
        for a in pk.pages[0].obj.get("/Annots", []):
            out[str(a.Contents)] = "/AP" in a and "/N" in a.AP
        return out


support = {n: bool(r.FPDFAnnot_IsSupportedSubtype(st)) for n, st in SUB.items()}
say("annot", "FPDFAnnot_IsSupportedSubtype: " + ", ".join(f"{k}={v}" for k, v in support.items()))
for mode in ["save_without_render", "render_then_save", "reload_render_save"]:
    d, _ = load(P("order.pdf"))
    page = r.FPDF_LoadPage(d, 0)
    made = make_annots(d, page)
    if mode == "render_then_save":
        b = white_bitmap(612, 792)
        r.FPDF_RenderPageBitmap(b, page, 0, 0, 612, 792, 0, r.FPDF_ANNOT)
        r.FPDFBitmap_Destroy(b)
    r.FPDF_ClosePage(page)
    path = write(f"annots_{mode}.pdf", save_bytes(d, r.FPDF_INCREMENTAL))
    r.FPDF_CloseDocument(d)
    if mode == "reload_render_save":
        d, _ = load(path)
        page = r.FPDF_LoadPage(d, 0)
        b = white_bitmap(612, 792)
        r.FPDF_RenderPageBitmap(b, page, 0, 0, 612, 792, 0, r.FPDF_ANNOT)
        r.FPDFBitmap_Destroy(b)
        r.FPDF_ClosePage(page)
        path = write(f"annots_{mode}.pdf", save_bytes(d, r.FPDF_INCREMENTAL))
        r.FPDF_CloseDocument(d)
    if mode == "save_without_render":
        say("annot", "FPDFPage_CreateAnnot result: " + ", ".join(f"{k}={'created' if v else 'NULL'}" for k, v in made.items()))
    say("annot", f"/AP /N written ({mode}): {ap_report(path)}")

# FPDFAnnot_SetAP: write our own appearance for a freetext annotation.
d, _ = load(P("order.pdf"))
page = r.FPDF_LoadPage(d, 0)
a = r.FPDFPage_CreateAnnot(page, r.FPDF_ANNOT_FREETEXT)
r.FPDFAnnot_SetRect(a, ctypes.byref(r.FS_RECTF(300, 400, 500, 370)))
r.FPDFAnnot_SetStringValue(a, b"Contents", wide("own AP"))
ok = r.FPDFAnnot_SetAP(a, r.FPDF_ANNOT_APPEARANCEMODE_NORMAL, wide("0 0 1 RG 1 w 300 370 200 30 re S"))
r.FPDFPage_CloseAnnot(a)
r.FPDF_ClosePage(page)
path = write("annot_setap.pdf", save_bytes(d, r.FPDF_INCREMENTAL))
r.FPDF_CloseDocument(d)
with pikepdf.open(path) as pk:
    ann = [x for x in pk.pages[0].Annots if str(x.Contents) == "own AP"][0]
    n = ann.AP.N
    say("annot", f"FPDFAnnot_SetAP ok={ok}: /AP/N keys={sorted(n.keys())} BBox={list(n.BBox)} stream={n.read_bytes()!r}")

# Signature as stamp image: image object inside a stamp annotation.
d, _ = load(P("order.pdf"))
page = r.FPDF_LoadPage(d, 0)
a = r.FPDFPage_CreateAnnot(page, r.FPDF_ANNOT_STAMP)
r.FPDFAnnot_SetRect(a, ctypes.byref(r.FS_RECTF(300, 300, 500, 250)))
img_bmp = r.FPDFBitmap_Create(200, 50, 1)  # BGRA with alpha, like a transparent signature PNG
r.FPDFBitmap_FillRect(img_bmp, 0, 0, 200, 50, 0x00FFFFFF)
r.FPDFBitmap_FillRect(img_bmp, 10, 20, 180, 4, 0xFF000080)
img = r.FPDFPageObj_NewImageObj(d)
ok_bmp = r.FPDFImageObj_SetBitmap(None, 0, img, img_bmp)
r.FPDFImageObj_SetMatrix(img, 200, 0, 0, 50, 300, 250)
ok_app = r.FPDFAnnot_AppendObject(a, img)
r.FPDFPage_CloseAnnot(a)
r.FPDF_ClosePage(page)
path = write("annot_signature_stamp.pdf", save_bytes(d, r.FPDF_INCREMENTAL))
r.FPDF_CloseDocument(d)
with pikepdf.open(path) as pk:
    ann = pk.pages[0].Annots[0]
    xo = ann.AP.N.Resources.XObject
    im = xo[list(xo.keys())[0]]
    say("sign", f"stamp+image: SetBitmap={ok_bmp} AppendObject={ok_app} /AP/N XObject /Subtype={im.Subtype} "
                f"/SMask={'present' if '/SMask' in im else 'absent'} {im.Width}x{im.Height}")

# ---------- 6. Pages ----------
d, _ = load(P("order.pdf"))
blank = r.FPDFPage_New(d, 1, 612, 792)
r.FPDF_ClosePage(blank)
jpg = open(P("photo.jpg"), "rb").read()
get_block_fn = dict(r.FPDF_FILEACCESS._fields_)["m_GetBlock"]


def get_block(_param, pos, pbuf, size):
    ctypes.memmove(pbuf, jpg[pos:pos + size], size)
    return 1


fa = r.FPDF_FILEACCESS(m_FileLen=len(jpg), m_Param=None)
fa.m_GetBlock = get_block_fn(get_block)
ip = r.FPDFPage_New(d, 2, 800 * 0.75, 600 * 0.75)
img = r.FPDFPageObj_NewImageObj(d)
arr = (r.FPDF_PAGE * 1)(ip)
ok_jpg = r.FPDFImageObj_LoadJpegFileInline(arr, 1, img, ctypes.byref(fa))
r.FPDFImageObj_SetMatrix(img, 600, 0, 0, 450, 0, 0)
r.FPDFPage_InsertObject(ip, img)
ok_gen = r.FPDFPage_GenerateContent(ip)
r.FPDFPage_SetRotation(ip, 1)
r.FPDFPage_SetCropBox(ip, 50, 50, 550, 400)
r.FPDF_ClosePage(ip)
order = (ctypes.c_int * 1)(2)
ok_move = r.FPDF_MovePages(d, order, 1, 0)  # image page to front
r.FPDFPage_Delete(d, 2)  # delete the blank page (now index 2)
fsrc, _ = load(P("form.pdf"))
ok_imp = r.FPDF_ImportPages(d, fsrc, b"1-2", r.FPDF_GetPageCount(d))  # merge form.pdf at the end
r.FPDF_CloseDocument(fsrc)
path = write("pages_edit.pdf", save_bytes(d, r.FPDF_NO_INCREMENTAL))
r.FPDF_CloseDocument(d)
with pikepdf.open(path) as pk:
    p0 = pk.pages[0]
    im = list(p0.Resources.XObject.values())[0]
    say("pages", f"New/LoadJpegFileInline={ok_jpg}/GenerateContent={ok_gen}/MovePages={ok_move}/ImportPages={ok_imp}; "
                 f"pages={len(pk.pages)}; page1 /Rotate={p0.get('/Rotate')} /CropBox={list(p0.CropBox)} "
                 f"image filter={im.Filter} raw bytes={len(im.read_raw_bytes())} (jpeg file {len(jpg)})")
    merged_widgets = sum(1 for p in pk.pages for a in p.obj.get("/Annots", []) if a.get("/Subtype") == "/Widget")
    say("pages", f"merge: widgets on merged pages={merged_widgets}; dest /AcroForm present={'/AcroForm' in pk.Root}")

big, _ = load(P("big500.pdf"))
new = r.FPDF_CreateNewDocument()
idx = (ctypes.c_int * 2)(0, 499)
ok = r.FPDF_ImportPagesByIndex(new, big, idx, 2, 0)
data = save_bytes(new, 0)
say("pages", f"extract pages 1 and 500 to new PDF: FPDF_ImportPagesByIndex={ok}, {len(data):,d} bytes")
r.FPDF_CloseDocument(new)
r.FPDF_CloseDocument(big)
