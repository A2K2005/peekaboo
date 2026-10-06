# Measures (non-reference PC) open + first-page render, tile render, thumbnails, and full-document search on a 500-page ~50 MB PDF.
# Usage: python perf.py <big500.pdf>
import ctypes, statistics, sys, time
import pypdfium2_raw as r
from pdfium_common import init, load, wide

src = sys.argv[1]
init()
ms = lambda t0: (time.perf_counter() - t0) * 1000


def stats(xs):
    return f"median {statistics.median(xs):7.1f} ms  min {min(xs):7.1f}  max {max(xs):7.1f}  (n={len(xs)})"


def render(page, w, h):
    bmp = r.FPDFBitmap_Create(w, h, 0)
    r.FPDFBitmap_FillRect(bmp, 0, 0, w, h, 0xFFFFFFFF)
    r.FPDF_RenderPageBitmap(bmp, page, 0, 0, w, h, 0, r.FPDF_ANNOT | r.FPDF_LCD_TEXT)
    r.FPDFBitmap_Destroy(bmp)


for label, (w, h) in {"100% scale 773x1000": (773, 1000), "150% scale 1160x1500": (1160, 1500)}.items():
    opens, loads, renders, totals = [], [], [], []
    for _ in range(9):
        t0 = time.perf_counter()
        doc, err = load(src)
        n = r.FPDF_GetPageCount(doc)
        t1 = time.perf_counter()
        page = r.FPDF_LoadPage(doc, 0)
        t2 = time.perf_counter()
        render(page, w, h)
        t3 = time.perf_counter()
        r.FPDF_ClosePage(page)
        r.FPDF_CloseDocument(doc)
        opens.append((t1 - t0) * 1000), loads.append((t2 - t1) * 1000), renders.append((t3 - t2) * 1000)
        totals.append((t3 - t0) * 1000)
    print(f"first page {label}: open+count {stats(opens)}")
    print(f"{'':32s}LoadPage   {stats(loads)}")
    print(f"{'':32s}render     {stats(renders)}")
    print(f"{'':32s}TOTAL      {stats(totals)}")

doc, _ = load(src)
page = r.FPDF_LoadPage(doc, 0)
times = []
for i in range(20):  # 512x512 tiles at 200% zoom across the page
    tx, ty = (i % 4) * 512, (i // 4 % 5) * 512
    bmp = r.FPDFBitmap_Create(512, 512, 0)
    r.FPDFBitmap_FillRect(bmp, 0, 0, 512, 512, 0xFFFFFFFF)
    t0 = time.perf_counter()
    r.FPDF_RenderPageBitmapWithMatrix(bmp, page, ctypes.byref(r.FS_MATRIX(2 * 96 / 72, 0, 0, 2 * 96 / 72, -tx, -ty)),
                                      ctypes.byref(r.FS_RECTF(0, 0, 512, 512)), r.FPDF_ANNOT)
    t = ms(t0)
    px = ctypes.string_at(r.FPDFBitmap_GetBuffer(bmp), 512 * 512 * 4)
    if px.count(b"\xff") < len(px):  # count only tiles that contain content
        times.append(t)
    r.FPDFBitmap_Destroy(bmp)
print(f"tile 512x512 at 200% with content (RenderPageBitmapWithMatrix): {stats(times)}")
r.FPDF_ClosePage(page)

t0 = time.perf_counter()
per = []
for i in range(r.FPDF_GetPageCount(doc)):
    t1 = time.perf_counter()
    p = r.FPDF_LoadPage(doc, i)
    render(p, 150, 194)
    r.FPDF_ClosePage(p)
    per.append(ms(t1))
print(f"thumbnails 150x194, 500 pages, one thread: total {ms(t0):.0f} ms, per page {stats(per)}")
r.FPDF_CloseDocument(doc)


def search(term, cache=None):
    doc, _ = load(src)
    hits, t0 = 0, time.perf_counter()
    q = wide(term)
    for i in range(r.FPDF_GetPageCount(doc)):
        p = r.FPDF_LoadPage(doc, i)
        tp = r.FPDFText_LoadPage(p)
        h = r.FPDFText_FindStart(tp, q, 0, 0)
        while r.FPDFText_FindNext(h):
            hits += 1
        r.FPDFText_FindClose(h)
        r.FPDFText_ClosePage(tp)
        r.FPDF_ClosePage(p)
    total = ms(t0)
    r.FPDF_CloseDocument(doc)
    return total, hits


for term in ["zebracrossing", "invoice"]:
    runs = [search(term) for _ in range(3)]
    print(f"search '{term}' over 500 pages (fresh doc, LoadPage+LoadTextPage+Find per page): "
          f"{stats([t for t, _ in runs])}, hits {runs[0][1]}")

# Text extraction once, then search in memory (what an app would cache after the first search).
doc, _ = load(src)
t0 = time.perf_counter()
texts = []
for i in range(r.FPDF_GetPageCount(doc)):
    p = r.FPDF_LoadPage(doc, i)
    tp = r.FPDFText_LoadPage(p)
    n = r.FPDFText_CountChars(tp)
    buf = (ctypes.c_ushort * (n + 1))()
    r.FPDFText_GetText(tp, 0, n, buf)
    texts.append(bytes(buf).decode("utf-16-le").lower())
    r.FPDFText_ClosePage(tp)
    r.FPDF_ClosePage(p)
t_extract = ms(t0)
t0 = time.perf_counter()
hits = sum(t.count("invoice") for t in texts)
print(f"extract all text once: {t_extract:.0f} ms ({sum(map(len, texts)):,d} chars); in-memory search 'invoice': {ms(t0):.1f} ms, hits {hits}")
r.FPDF_CloseDocument(doc)
