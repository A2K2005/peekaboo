# Proves nothing by itself: generates the test PDFs (500-page ~50 MB, AcroForm, XFA marker, password) that the other proofs use.
# Usage: python make_test_pdfs.py <out_dir>
import os, random, sys
import pikepdf
from pikepdf import Array, Dictionary, Name, Pdf, Stream, String

out = sys.argv[1]
os.makedirs(out, exist_ok=True)
rnd = random.Random(42)
WORDS = ("invoice account balance payment total amount tax period customer service report annual "
         "statement policy contract clause section page figure table summary review value market").split()


def helv(pdf):
    return pdf.make_indirect(Dictionary(Type=Name.Font, Subtype=Name.Type1, BaseFont=Name.Helvetica,
                                        Encoding=Name.WinAnsiEncoding))


def big_pdf(path, pages=500, img_side=185):
    # Each page: ~45 lines of text plus one incompressible RGB image (~100 KB) so the file is ~50 MB.
    pdf = Pdf.new()
    font = helv(pdf)
    for i in range(pages):
        lines = []
        for _ in range(45):
            lines.append(" ".join(rnd.choice(WORDS) for _ in range(11)))
        if i == pages - 1:
            lines[20] = "the unique needle phrase zebracrossing appears only here"
        text = "BT /F1 10 Tf 50 760 Td 12 TL " + " ".join(f"({l}) '" for l in lines) + " ET\n"
        img = Stream(pdf, os.urandom(img_side * img_side * 3))
        img.Type, img.Subtype = Name.XObject, Name.Image
        img.Width = img.Height = img_side
        img.ColorSpace, img.BitsPerComponent = Name.DeviceRGB, 8
        content = text + f"q 150 0 0 150 400 40 cm /Im0 Do Q\n"
        page = pdf.add_blank_page(page_size=(612, 792))
        page.Resources = Dictionary(Font=Dictionary(F1=font), XObject=Dictionary(Im0=img))
        page.Contents = pdf.make_stream(content.encode("latin-1"))
    # Uncompressed image data stays ~100 KB/page; compress text only.
    pdf.save(path, compress_streams=True, object_stream_mode=pikepdf.ObjectStreamMode.disable)


def form_pdf(path):
    # Two pages. Page 1: three text fields. Page 2: one text field and one checkbox.
    pdf = Pdf.new()
    font = helv(pdf)
    p1 = pdf.add_blank_page(page_size=(612, 792))
    p2 = pdf.add_blank_page(page_size=(612, 792))
    def text_field(page, name, rect):
        return pdf.make_indirect(Dictionary(Type=Name.Annot, Subtype=Name.Widget, FT=Name.Tx, T=String(name),
                                            Rect=Array(rect), F=4, DA=String("/Helv 12 Tf 0 g"), P=page.obj,
                                            MK=Dictionary(BC=Array([0, 0, 0]))))

    cb = pdf.make_indirect(Dictionary(Type=Name.Annot, Subtype=Name.Widget, FT=Name.Btn, T=String("agree"),
                                      Rect=Array([100, 650, 118, 668]), F=4, V=Name.Off, AS=Name.Off, P=p2.obj,
                                      DA=String("/ZaDb 0 Tf 0 g"), MK=Dictionary(CA=String("4"))))
    a1 = [text_field(p1, "name", [100, 700, 400, 724]), text_field(p1, "email", [100, 650, 400, 674]),
          text_field(p1, "city", [100, 600, 400, 624])]
    a2 = [text_field(p2, "notes", [100, 700, 400, 724]), cb]
    p1.Annots, p2.Annots = Array(a1), Array(a2)
    fields = a1 + a2
    zadb = pdf.make_indirect(Dictionary(Type=Name.Font, Subtype=Name.Type1, BaseFont=Name.ZapfDingbats))
    pdf.Root.AcroForm = Dictionary(Fields=Array(fields), DA=String("/Helv 12 Tf 0 g"),
                                   DR=Dictionary(Font=Dictionary(Helv=font, ZaDb=zadb)))
    pdf.save(path)


def xfa_pdf(path, full):
    # Minimal AcroForm with an /XFA packet. full=True sets /NeedsRendering (XFA full / dynamic form).
    pdf = Pdf.new()
    pdf.add_blank_page(page_size=(612, 792))
    xdp = pdf.make_stream(b'<xdp:xdp xmlns:xdp="http://ns.adobe.com/xdp/"><template '
                          b'xmlns="http://www.xfa.org/schema/xfa-template/3.3/"/></xdp:xdp>')
    pdf.Root.AcroForm = Dictionary(Fields=Array(), XFA=xdp)
    if full:
        pdf.Root.NeedsRendering = True
    pdf.save(path)


def order_pdf(path):
    # Two columns. The content stream draws the RIGHT column first, and the left column bottom-up.
    # Visual reading order is L1 L2 L3 R1 R2 R3.
    pdf = Pdf.new()
    page = pdf.add_blank_page(page_size=(612, 792))
    page.Resources = Dictionary(Font=Dictionary(F1=helv(pdf)))
    ops = ["BT /F1 12 Tf 320 700 Td (R1 right top) Tj ET", "BT /F1 12 Tf 320 680 Td (R2 right middle) Tj ET",
           "BT /F1 12 Tf 320 660 Td (R3 right bottom) Tj ET", "BT /F1 12 Tf 50 660 Td (L3 left bottom) Tj ET",
           "BT /F1 12 Tf 50 680 Td (L2 left middle) Tj ET", "BT /F1 12 Tf 50 700 Td (L1 left top) Tj ET"]
    page.Contents = pdf.make_stream("\n".join(ops).encode())
    pdf.save(path)


def jpeg(path):
    from PIL import Image
    Image.effect_noise((800, 600), 64).convert("RGB").save(path, quality=85)


def password_pdf(src, path):
    with Pdf.open(src) as pdf:
        pdf.save(path, encryption=pikepdf.Encryption(user="user123", owner="owner456", R=6))


if __name__ == "__main__":
    big = os.path.join(out, "big500.pdf")
    if not os.path.exists(big):
        big_pdf(big)
    form_pdf(os.path.join(out, "form.pdf"))
    xfa_pdf(os.path.join(out, "xfa_full.pdf"), True)
    xfa_pdf(os.path.join(out, "xfa_foreground.pdf"), False)
    password_pdf(os.path.join(out, "form.pdf"), os.path.join(out, "password_aes256.pdf"))
    order_pdf(os.path.join(out, "order.pdf"))
    jpeg(os.path.join(out, "photo.jpg"))
    for n in sorted(os.listdir(out)):
        if n.endswith(".pdf"):
            print(f"{n:28s} {os.path.getsize(os.path.join(out, n)):>12,d} bytes")
