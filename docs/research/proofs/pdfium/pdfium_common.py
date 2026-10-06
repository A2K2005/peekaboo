# Shared helpers for the PDFium proofs. Calls the raw PDFium C API through pypdfium2_raw (ctypes), the same calls C# would make.
import ctypes
import pypdfium2_raw as r

_WRITE_FN = dict(r.FPDF_FILEWRITE._fields_)["WriteBlock"]


def init():
    cfg = r.FPDF_LIBRARY_CONFIG(version=2, m_pUserFontPaths=None, m_pIsolate=None, m_v8EmbedderSlot=0)
    r.FPDF_InitLibraryWithConfig(ctypes.byref(cfg))


def load(path, password=None):
    """Returns (doc_or_None, FPDF_GetLastError())."""
    doc = r.FPDF_LoadDocument(str(path).encode("utf-8"), password.encode("utf-8") if password else None)
    return (doc if doc else None), r.FPDF_GetLastError()


def save_bytes(doc, flags):
    """FPDF_SaveAsCopy into memory. Returns the saved bytes, or raises."""
    chunks = []

    def write(_self, data, size):
        chunks.append(ctypes.string_at(data, size))
        return 1

    w = r.FPDF_FILEWRITE(version=1)
    w.WriteBlock = _WRITE_FN(write)
    if not r.FPDF_SaveAsCopy(doc, ctypes.byref(w), flags):
        raise RuntimeError("FPDF_SaveAsCopy failed")
    return b"".join(chunks)


def page_text(textpage):
    n = r.FPDFText_CountChars(textpage)
    buf = (ctypes.c_ushort * (n + 1))()
    got = r.FPDFText_GetText(textpage, 0, n, buf)
    return bytes(buf)[: max(got - 1, 0) * 2].decode("utf-16-le")


def wide(s):
    """UTF-16LE, NUL-terminated, as FPDF_WIDESTRING."""
    b = (s + "\0").encode("utf-16-le")
    return ctypes.cast(ctypes.create_string_buffer(b, len(b)), ctypes.POINTER(ctypes.c_ushort))
