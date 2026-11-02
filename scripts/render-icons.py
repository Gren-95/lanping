#!/usr/bin/env python3
"""Renders packaging/lanping.svg to the icons for the app, the Windows exe and macOS.

Run after changing the SVG. Needs the cairosvg and Pillow packages.
"""
import io
from pathlib import Path

import cairosvg
from PIL import Image

WINDOW_ICON_SIZE = 64
ICO_SIZE = 256
ICNS_SIZE = 1024
ICO_SIZES = [(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]

root = Path(__file__).resolve().parent.parent
svg = str(root / "packaging/lanping.svg")


def render(size):
    png = cairosvg.svg2png(url=svg, output_width=size, output_height=size)
    return Image.open(io.BytesIO(png)).convert("RGBA")


(root / "crates/lanping/assets/window-icon-64.rgba").write_bytes(render(WINDOW_ICON_SIZE).tobytes())
render(ICO_SIZE).save(root / "packaging/lanping.ico", sizes=ICO_SIZES)
render(ICNS_SIZE).save(root / "packaging/lanping.icns")
