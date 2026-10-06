# Proves pdfium.dll exports no encrypt-on-save API, and that qpdf (CLI and C API, official 12.4.2 build) adds AES-256 encryption PDFium can open.
# Usage: python encrypt_gap.py <pdfium.dll> <qpdf_bin_dir> <in.pdf> <out_dir>
import ctypes, os, struct, subprocess, sys, time
import pypdfium2_raw as r
from pdfium_common import init, load

pdfium_dll, qbin, src, out = sys.argv[1:5]


def pe_exports(path):
    """Names in a PE file's export table."""
    b = open(path, "rb").read()
    pe = struct.unpack_from("<I", b, 0x3C)[0]
    nsec = struct.unpack_from("<H", b, pe + 6)[0]
    opt = pe + 24
    exp_rva = struct.unpack_from("<I", b, opt + 112)[0]  # PE32+ data directory 0
    secs = [struct.unpack_from("<IIII", b, opt + struct.unpack_from("<H", b, pe + 20)[0] + 40 * i + 8)
            for i in range(nsec)]  # (vsize, vaddr, rawsize, rawptr)

    def off(rva):
        for vs, va, rs, rp in secs:
            if va <= rva < va + max(vs, rs):
                return rva - va + rp
    e = off(exp_rva)
    n_names, names_rva = struct.unpack_from("<I", b, e + 24)[0], struct.unpack_from("<I", b, e + 32)[0]
    names = []
    for i in range(n_names):
        p = off(struct.unpack_from("<I", b, off(names_rva) + 4 * i)[0])
        names.append(b[p:b.index(b"\0", p)].decode())
    return names


names = pe_exports(pdfium_dll)
hits = [n for n in names if any(k in n.lower() for k in ("encrypt", "password", "secur", "permission"))]
print(f"pdfium.dll exports: {len(names)}; names containing encrypt/password/secur/permission: {hits}")

init()


def verify(path):
    d0, e0 = load(path)
    d1, e1 = load(path, "user123")
    rev = r.FPDF_GetSecurityHandlerRevision(d1) if d1 else None
    pages = r.FPDF_GetPageCount(d1) if d1 else None
    for d in (d0, d1):
        if d:
            r.FPDF_CloseDocument(d)
    return f"PDFium: no password -> err {e0}; user password -> {'opens' if d1 else e1}, revision {rev}, pages {pages}"


# 1. qpdf CLI
o1 = os.path.join(out, "enc_cli.pdf")
t = time.perf_counter()
cp = subprocess.run([os.path.join(qbin, "qpdf.exe"), "--encrypt", "--user-password=user123",
                     "--owner-password=owner456", "--bits=256", "--", src, o1], capture_output=True, text=True)
print(f"qpdf CLI exit {cp.returncode} in {(time.perf_counter() - t) * 1000:.0f} ms; {verify(o1)}")

# 2. qpdf C API (qpdf-c.h) through ctypes
os.add_dll_directory(qbin)
q = ctypes.CDLL(os.path.join(qbin, "qpdf30.dll"))
q.qpdf_init.restype = ctypes.c_void_p
q.qpdf_read.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_char_p]
q.qpdf_init_write.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
q.qpdf_write.argtypes = [ctypes.c_void_p]
q.qpdf_set_r6_encryption_parameters2.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_char_p] + [ctypes.c_int] * 8
q.qpdf_cleanup.argtypes = [ctypes.POINTER(ctypes.c_void_p)]
q.qpdf_get_qpdf_version.restype = ctypes.c_char_p
o2 = os.path.join(out, "enc_capi.pdf")
t = time.perf_counter()
h = ctypes.c_void_p(q.qpdf_init())
rc = [q.qpdf_read(h, src.encode(), None), q.qpdf_init_write(h, o2.encode())]
#                                            user        owner     access extract assemble annotate form modify print(0=full) meta
q.qpdf_set_r6_encryption_parameters2(h, b"user123", b"owner456", 1, 1, 0, 1, 1, 0, 0, 1)
rc.append(q.qpdf_write(h))
q.qpdf_cleanup(ctypes.byref(h))
print(f"qpdf C API {q.qpdf_get_qpdf_version().decode()}: return codes {rc} in {(time.perf_counter() - t) * 1000:.0f} ms; {verify(o2)}")
