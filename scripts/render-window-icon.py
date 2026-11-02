#!/usr/bin/env python3
"""Renders packaging/lanping.svg to the raw RGBA window icon built into the app.

Run after changing the SVG. Needs the cairosvg and Pillow packages.
"""
import io
from pathlib import Path

import cairosvg
from PIL import Image

SIZE = 64
root = Path(__file__).resolve().parent.parent
png = cairosvg.svg2png(url=str(root / "packaging/lanping.svg"), output_width=SIZE, output_height=SIZE)
rgba = Image.open(io.BytesIO(png)).convert("RGBA").tobytes()
(root / "crates/lanping/assets/window-icon-64.rgba").write_bytes(rgba)
