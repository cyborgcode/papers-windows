"""Generate small sample documents for the Windows smoke test.

Writes, into the output directory: sample.pdf (outline, link, note annotation,
form field, metadata with XMP), sample.tiff (multi-page), sample.cbz and
sample.cbt (comic archives of PNG pages), and page PPMs used to build a DjVu
file with DjVuLibre's c44/djvm.

Usage: make-samples.py OUTPUT_DIR
"""
import io
import os
import sys
import tarfile
import zipfile

from PIL import Image, ImageDraw

out = sys.argv[1]
os.makedirs(out, exist_ok=True)

COLORS = [(52, 101, 164), (78, 154, 6), (204, 0, 0)]


def page_image(n, size=(850, 1100)):
    im = Image.new("RGB", size, "white")
    d = ImageDraw.Draw(im)
    d.rectangle([40, 40, size[0] - 40, 200], fill=COLORS[(n - 1) % 3])
    d.text((60, 100), f"Papers for Windows - sample page {n}", fill="white")
    for i in range(20):
        d.line([60, 260 + i * 36, size[0] - 60, 260 + i * 36], fill=(180, 180, 180), width=2)
    return im


pages = [page_image(n) for n in (1, 2, 3)]

# Multi-page TIFF
pages[0].save(os.path.join(out, "sample.tiff"), save_all=True, append_images=pages[1:], compression="tiff_deflate")


def png_bytes(im):
    buf = io.BytesIO()
    im.save(buf, format="PNG")
    return buf.getvalue()


# Comic book archives
with zipfile.ZipFile(os.path.join(out, "sample.cbz"), "w") as z:
    for i, im in enumerate(pages, 1):
        z.writestr(f"page{i:02d}.png", png_bytes(im))
with tarfile.open(os.path.join(out, "sample.cbt"), "w") as t:
    for i, im in enumerate(pages, 1):
        data = png_bytes(im)
        info = tarfile.TarInfo(f"page{i:02d}.png")
        info.size = len(data)
        t.addfile(info, io.BytesIO(data))

# Loose PNG pages (packed into a .cb7 with bsdtar)
os.makedirs(os.path.join(out, "pages"), exist_ok=True)
for i, im in enumerate(pages, 1):
    im.save(os.path.join(out, "pages", f"page{i:02d}.png"))

# Pages for DjVu encoding (c44 reads PNM)
for i, im in enumerate(pages, 1):
    im.save(os.path.join(out, f"djvu-page{i}.ppm"))


# Hand-written PDF exercising the features Papers' sidebars and dialogs show
def pdf():
    objs = {}

    def obj(num, body):
        objs[num] = body if isinstance(body, bytes) else body.encode("latin-1")

    xmp = (
        '<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>'
        '<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">'
        '<rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/" '
        'xmlns:xmpRights="http://ns.adobe.com/xap/1.0/rights/" xmpRights:Marked="True">'
        '<dc:title><rdf:Alt><rdf:li xml:lang="x-default">Papers Windows Sample</rdf:li></rdf:Alt></dc:title>'
        '<dc:creator><rdf:Seq><rdf:li>Papers CI</rdf:li></rdf:Seq></dc:creator>'
        '<xmpRights:UsageTerms><rdf:Alt><rdf:li xml:lang="x-default">CC BY 4.0</rdf:li></rdf:Alt></xmpRights:UsageTerms>'
        "</rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end=\"w\"?>"
    ).encode()

    obj(1, "<< /Type /Catalog /Pages 2 0 R /Outlines 20 0 R /PageMode /UseOutlines /AcroForm 30 0 R /Metadata 40 0 R >>")
    obj(2, "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>")
    for i, num in enumerate((3, 4, 5)):
        annots = ""
        if num == 3:
            annots = "/Annots [10 0 R 11 0 R 31 0 R]"
        obj(num, f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {6 + i} 0 R "
                 f"/Resources << /Font << /F1 9 0 R >> >> {annots} >>")
        text = f"BT /F1 28 Tf 72 700 Td (Papers for Windows - page {i + 1}) Tj ET"
        if num == 3:
            text += " BT /F1 14 Tf 72 650 Td (Go to page 3) Tj ET"
        obj(6 + i, f"<< /Length {len(text)} >>\nstream\n{text}\nendstream")
    obj(9, "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
    obj(10, "<< /Type /Annot /Subtype /Link /Rect [70 640 200 668] /Border [0 0 0] /Dest [5 0 R /XYZ 0 792 0] >>")
    obj(11, "<< /Type /Annot /Subtype /Text /Rect [400 600 424 624] /Contents (A note made on Windows) "
             "/T (Papers CI) /C [1 0.9 0.2] /Name /Comment >>")
    obj(20, "<< /Type /Outlines /First 21 0 R /Last 22 0 R /Count 2 >>")
    obj(21, "<< /Title (Chapter 1) /Parent 20 0 R /Next 22 0 R /Dest [3 0 R /Fit] >>")
    obj(22, "<< /Title (Chapter 2) /Parent 20 0 R /Prev 21 0 R /Dest [4 0 R /Fit] >>")
    obj(30, "<< /Fields [31 0 R] /NeedAppearances true /DA (/Helv 12 Tf 0 g) "
             "/DR << /Font << /Helv 9 0 R >> >> >>")
    obj(31, "<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /V (Fill me) /Rect [72 560 300 584] "
             "/P 3 0 R /DA (/Helv 12 Tf 0 g) /F 4 >>")
    obj(40, b"<< /Type /Metadata /Subtype /XML /Length " + str(len(xmp)).encode() + b" >>\nstream\n" + xmp + b"\nendstream")
    obj(41, "<< /Title (Papers Windows Sample) /Author (Papers CI) /Subject (Smoke test) "
             "/Keywords (windows, papers) /Creator (make-samples.py) /Producer (hand-written) "
             "/CreationDate (D:20261014120000Z) >>")

    data = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = {}
    for num in sorted(objs):
        offsets[num] = len(data)
        data += f"{num} 0 obj\n".encode() + objs[num] + b"\nendobj\n"
    size = max(objs) + 1
    xref = len(data)
    data += f"xref\n0 {size}\n".encode() + b"0000000000 65535 f \n"
    for num in range(1, size):
        if num in offsets:
            data += f"{offsets[num]:010d} 00000 n \n".encode()
        else:
            data += b"0000000000 65535 f \n"
    data += f"trailer\n<< /Size {size} /Root 1 0 R /Info 41 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode()
    return bytes(data)


with open(os.path.join(out, "sample.pdf"), "wb") as f:
    f.write(pdf())

print("samples written to", out)
