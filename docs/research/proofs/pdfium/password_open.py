# Proves how PDFium opens password PDFs (error codes for none/wrong/right password) and what FPDF_SaveAsCopy does to existing encryption.
# Usage: python password_open.py <form.pdf> <out_dir>
import ctypes, os, sys
import pikepdf
import pypdfium2_raw as r
from pdfium_common import init, load, save_bytes

src, out = sys.argv[1], sys.argv[2]
init()
ERR = {0: "SUCCESS", 1: "UNKNOWN", 2: "FILE", 3: "FORMAT", 4: "PASSWORD", 5: "SECURITY", 6: "PAGE"}
variants = {  # name: pikepdf.Encryption (pikepdf = libqpdf)
    "R6_AES256": pikepdf.Encryption(user="user123", owner="owner456", R=6),
    "R4_AES128": pikepdf.Encryption(user="user123", owner="owner456", R=4, aes=True),
    "R3_RC4_128": pikepdf.Encryption(user="user123", owner="owner456", R=3, aes=False, metadata=False),
    "R6_owner_only": pikepdf.Encryption(user="", owner="owner456", R=6),
}


def add_highlight(doc):
    page = r.FPDF_LoadPage(doc, 0)
    a = r.FPDFPage_CreateAnnot(page, r.FPDF_ANNOT_HIGHLIGHT)
    r.FPDFAnnot_AppendAttachmentPoints(a, ctypes.byref(r.FS_QUADPOINTSF(100, 724, 400, 724, 100, 700, 400, 700)))
    r.FPDFPage_CloseAnnot(a)
    r.FPDF_ClosePage(page)


def state(path):
    """How a reader sees a saved file: needs password? opens with it? revision."""
    d0, e0 = load(path)
    d1, e1 = load(path, "user123")
    rev = r.FPDF_GetSecurityHandlerRevision(d1 or d0) if (d1 or d0) else None
    for d in (d0, d1):
        if d:
            r.FPDF_CloseDocument(d)
    return f"no-pw:{ERR[e0] if not d0 else 'opens'} user-pw:{ERR[e1] if not d1 else 'opens'} rev:{rev}"


for name, enc in variants.items():
    path = os.path.join(out, f"pw_{name}.pdf")
    with pikepdf.open(src) as p:
        p.save(path, encryption=enc)
    print(f"== {name}")
    for label, pw in [("no password", None), ("wrong password", "nope"), ("user password", "user123"),
                      ("owner password", "owner456")]:
        doc, err = load(path, pw)
        extra = ""
        if doc:
            extra = (f" rev={r.FPDF_GetSecurityHandlerRevision(doc)} perms=0x{r.FPDF_GetDocPermissions(doc) & 0xFFFFFFFF:08X}"
                     f" user_perms=0x{r.FPDF_GetDocUserPermissions(doc) & 0xFFFFFFFF:08X}")
            r.FPDF_CloseDocument(doc)
        print(f"  open with {label:15s}: {'OK' if doc else 'FAIL'} FPDF_GetLastError={err} ({ERR.get(err)}){extra}")
    for flag_name, flags in [("INCREMENTAL", r.FPDF_INCREMENTAL), ("NO_INCREMENTAL", r.FPDF_NO_INCREMENTAL),
                             ("REMOVE_SECURITY", r.FPDF_REMOVE_SECURITY)]:
        doc, _ = load(path, "user123" if "owner_only" not in name else None)
        add_highlight(doc)
        data = save_bytes(doc, flags)
        r.FPDF_CloseDocument(doc)
        o = os.path.join(out, f"pw_{name}_{flag_name}.pdf")
        open(o, "wb").write(data)
        orig = open(path, "rb").read()
        print(f"  save {flag_name:15s}: appended={data[:len(orig)] == orig} -> {state(o)}")
