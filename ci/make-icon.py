"""Assemble a multi-resolution Windows .ico from PNG renders of the app icon.

Usage: make-icon.py OUTPUT.ico INPUT-16.png INPUT-24.png ... (largest last)
"""
import sys

from PIL import Image

out, *pngs = sys.argv[1:]
images = [Image.open(p).convert("RGBA") for p in pngs]
largest = max(images, key=lambda im: im.size[0])
largest.save(out, format="ICO", sizes=[im.size for im in images], append_images=images)
