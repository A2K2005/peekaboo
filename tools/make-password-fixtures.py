"""Create test-only encrypted PDFs with the already installed pypdf package.

Run make-fixtures.ps1 first. Credentials below belong only to generated synthetic
fixtures, never to user documents. pypdf is not an application dependency.
API source: https://pypdf.readthedocs.io/en/stable/modules/PdfWriter.html
"""
from pathlib import Path
from pypdf import PdfReader, PdfWriter

root = Path(__file__).resolve().parents[1] / "fixtures"
for name, permissions in [("password.pdf", 4294967292), ("password-restricted.pdf", 0)]:
    writer = PdfWriter()
    writer.add_page(PdfReader(root / "20-pages.pdf").pages[0])
    writer.encrypt("preview-test", "owner-test", permissions_flag=permissions, algorithm="AES-256")
    with (root / name).open("wb") as output:
        writer.write(output)
print("Created two synthetic AES-256 PDF fixtures.")
