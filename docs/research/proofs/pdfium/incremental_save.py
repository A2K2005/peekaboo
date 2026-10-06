# Proves FPDF_SaveAsCopy(FPDF_INCREMENTAL) keeps the original bytes and only appends, for a highlight, a page move, and a page delete.
# Usage: python incremental_save.py <big.pdf> <out_dir>
import ctypes, hashlib, os, sys, time
import pikepdf
import pypdfium2_raw as r
from pdfium_common import init, load, save_bytes

src, out_dir = sys.argv[1], sys.argv[2]
orig = open(src, "rb").read()
init()


def add_highlight(doc, page_index=0):
    page = r.FPDF_LoadPage(doc, page_index)
    annot = r.FPDFPage_CreateAnnot(page, r.FPDF_ANNOT_HIGHLIGHT)
    assert annot
    q = r.FS_QUADPOINTSF(50, 760, 300, 760, 50, 748, 300, 748)
    assert r.FPDFAnnot_AppendAttachmentPoints(annot, ctypes.byref(q))
    assert r.FPDFAnnot_SetRect(annot, ctypes.byref(r.FS_RECTF(50, 760, 300, 748)))
    assert r.FPDFAnnot_SetColor(annot, r.FPDFANNOT_COLORTYPE_Color, 255, 230, 0, 128)
    r.FPDFPage_CloseAnnot(annot)
    r.FPDF_ClosePage(page)


def render_all(doc):
    # Simulates a user who scrolled through every page before editing.
    bmp = r.FPDFBitmap_Create(612, 792, 0)
    for i in range(r.FPDF_GetPageCount(doc)):
        p = r.FPDF_LoadPage(doc, i)
        r.FPDF_RenderPageBitmap(bmp, p, 0, 0, 612, 792, 0, r.FPDF_ANNOT)
        r.FPDF_ClosePage(p)
    r.FPDFBitmap_Destroy(bmp)


def move_last_to_first(doc):
    idx = (ctypes.c_int * 1)(r.FPDF_GetPageCount(doc) - 1)
    assert r.FPDF_MovePages(doc, idx, 1, 0)


def delete_page_10(doc):
    r.FPDFPage_Delete(doc, 10)


def check(name, data):
    path = os.path.join(out_dir, name + ".pdf")
    open(path, "wb").write(data)
    prefix_same = data[: len(orig)] == orig
    with pikepdf.open(path) as p:  # qpdf parse = independent validity check
        n_pages = len(p.pages)
        annots0 = len(p.pages[0].obj.get("/Annots", []))
        has_ap = any("/AP" in a for a in p.pages[0].obj.get("/Annots", []))
    doc, err = load(path)
    pdfium_pages = r.FPDF_GetPageCount(doc)
    r.FPDF_CloseDocument(doc)
    return prefix_same, n_pages, pdfium_pages, annots0, has_ap


cases = [
    ("A_highlight_incremental", [add_highlight], r.FPDF_INCREMENTAL),
    ("B_render_all_then_highlight_incremental", [render_all, add_highlight], r.FPDF_INCREMENTAL),
    ("C_move_last_to_first_incremental", [move_last_to_first], r.FPDF_INCREMENTAL),
    ("D_delete_page_10_incremental", [delete_page_10], r.FPDF_INCREMENTAL),
    ("E_highlight_full_rewrite", [add_highlight], r.FPDF_NO_INCREMENTAL),
]
print(f"source: {len(orig):,d} bytes, sha256 {hashlib.sha256(orig).hexdigest()[:16]}")
print(f"{'case':42s} {'save ms':>8s} {'out bytes':>12s} {'added':>11s} prefix_same qpdf_pages pdfium_pages annots_p0 /AP")
for name, edits, flags in cases:
    doc, err = load(src)
    assert doc, err
    for e in edits:
        e(doc)
    t = time.perf_counter()
    data = save_bytes(doc, flags)
    ms = (time.perf_counter() - t) * 1000
    r.FPDF_CloseDocument(doc)
    prefix_same, n, pn, a0, ap = check(name, data)
    print(f"{name:42s} {ms:8.1f} {len(data):12,d} {len(data) - len(orig):11,d} {str(prefix_same):11s} {n:10d} {pn:12d} {a0:9d} {ap}")
    if name.startswith("A_"):
        tail = data[len(orig):]
        print("  appended section (first 600 bytes):\n" + tail[:600].decode("latin-1"))
