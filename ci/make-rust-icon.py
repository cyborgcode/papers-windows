"""Draw the Papers icon: a page that is half day, half night, on a blue tile.

Writes packaging/papers.ico (16-256 px) and apps/papers/assets/icon-256.png.
Usage: python ci/make-rust-icon.py
"""
import os

from PIL import Image, ImageDraw

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
S = 1024  # draw large, then downsample for smooth edges


def draw():
    im = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    d.rounded_rectangle([40, 40, S - 40, S - 40], radius=200, fill=(46, 104, 196, 255))
    # page with folded corner
    x0, y0, x1, y1, fold = 250, 160, 774, 864, 150
    page = [(x0, y0), (x1 - fold, y0), (x1, y0 + fold), (x1, y1), (x0, y1)]
    d.polygon([(x + 14, y + 18) for x, y in page], fill=(18, 40, 80, 110))  # shadow
    d.polygon(page, fill=(250, 250, 248, 255))
    # night half: right side of the page
    mid = (x0 + x1) // 2
    night = [(mid, y0), (x1 - fold, y0), (x1, y0 + fold), (x1, y1), (mid, y1)]
    d.polygon(night, fill=(36, 38, 46, 255))
    d.polygon([(x1 - fold, y0), (x1 - fold, y0 + fold), (x1, y0 + fold)], fill=(120, 126, 140, 255))
    # text lines: dark on the day side, light on the night side
    for i, y in enumerate(range(y0 + 230, y1 - 80, 74)):
        right = x1 - 70 if i % 3 != 2 else x1 - 200
        d.rounded_rectangle([x0 + 60, y, mid, y + 30], radius=15, fill=(60, 64, 76, 255))
        d.rounded_rectangle([mid, y, right, y + 30], radius=15, fill=(214, 218, 228, 255))
    # moon on the night side
    d.ellipse([mid + 50, y0 + 70, mid + 170, y0 + 190], fill=(255, 214, 102, 255))
    d.ellipse([mid + 92, y0 + 52, mid + 205, y0 + 165], fill=(36, 38, 46, 255))
    return im


big = draw()
sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
images = [big.resize((s, s), Image.LANCZOS) for s in sizes]
images[-1].save(os.path.join(ROOT, "apps", "papers", "assets", "icon-256.png"))
images[-1].save(os.path.join(ROOT, "packaging", "papers.ico"), format="ICO", sizes=[(s, s) for s in sizes], append_images=images[:-1])
print("icon written")
