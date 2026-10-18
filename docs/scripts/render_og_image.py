#!/usr/bin/env python3
"""Rasterise the claimdag logo into the 1200x630 share card.

Reads docs/source/_static/logo.svg (and the mark it embeds) and writes
docs/source/_static/og.png. Requires rsvg-convert from librsvg.

    python3 docs/scripts/render_og_image.py
"""

from __future__ import annotations

import re
import shutil
import struct
import subprocess
import sys
import tempfile
import zlib
from pathlib import Path

_STATIC = Path(__file__).resolve().parents[1] / "source" / "_static"
_LOGO = _STATIC / "logo.svg"
_MARK = _STATIC / "mark.svg"
_OUT = _STATIC / "og.png"

_CANVAS = (1200, 630)
_PAD = 72
_BG = "#0B0B0A"
# The logo names Iowan Old Style. Liberation Serif is the face this
# raster uses so the card does not depend on a font the build host lacks.
_FONT = "Liberation Serif, Noto Serif, serif"


def _inner(svg: str) -> str:
    inner = re.sub(r"^.*?<svg\b[^>]*>", "", svg, count=1, flags=re.S)
    inner = re.sub(r"</svg>\s*$", "", inner)
    return inner.strip()


def _viewbox(svg: str) -> str:
    match = re.search(r'\bviewBox="([^"]+)"', svg)
    if not match:
        raise SystemExit(f"svg has no viewBox: {_LOGO.name}")
    return match.group(1)


def _ink_box(png: Path, background: tuple[int, int, int] = (0x0B, 0x0B, 0x0A)) -> tuple[int, int, int, int]:
    """Pixel bounds of anything that is not the logo's own ground."""
    data = png.read_bytes()
    pos = 8
    width = height = color = 0
    idat = b""
    while pos < len(data):
        length, kind = struct.unpack(">I4s", data[pos : pos + 8])
        pos += 8
        chunk = data[pos : pos + length]
        pos += length + 4
        if kind == b"IHDR":
            width, height, _bit, color = struct.unpack(">IIBB", chunk[:10])
        elif kind == b"IDAT":
            idat += chunk
        elif kind == b"IEND":
            break
    raw = zlib.decompress(idat)
    channels = {2: 3, 6: 4}[color]
    stride = width * channels
    prev = bytearray(stride)
    offset = 0
    min_x, min_y, max_x, max_y = width, height, -1, -1

    def paeth(left: int, up: int, up_left: int) -> int:
        estimate = left + up - up_left
        da, db, dc = abs(estimate - left), abs(estimate - up), abs(estimate - up_left)
        if da <= db and da <= dc:
            return left
        return up if db <= dc else up_left

    for y in range(height):
        filt = raw[offset]
        offset += 1
        row = bytearray(raw[offset : offset + stride])
        offset += stride
        for x in range(stride):
            left = row[x - channels] if x >= channels else 0
            up = prev[x]
            up_left = prev[x - channels] if x >= channels else 0
            if filt == 1:
                row[x] = (row[x] + left) & 255
            elif filt == 2:
                row[x] = (row[x] + up) & 255
            elif filt == 3:
                row[x] = (row[x] + ((left + up) // 2)) & 255
            elif filt == 4:
                row[x] = (row[x] + paeth(left, up, up_left)) & 255
            elif filt != 0:
                raise SystemExit(f"render_og_image: unknown png filter {filt}")
        for x in range(width):
            pixel = row[x * channels : (x + 1) * channels]
            if any(abs(pixel[channel] - background[channel]) > 12 for channel in range(3)):
                min_x = min(min_x, x)
                min_y = min(min_y, y)
                max_x = max(max_x, x)
                max_y = max(max_y, y)
        prev = row
    if max_x < 0:
        raise SystemExit("render_og_image: logo raster is empty")
    return min_x, min_y, max_x, max_y


def _card_svg(rsvg: str, tmp: Path) -> str:
    logo = _LOGO.read_text(encoding="utf-8")
    mark = _MARK.read_text(encoding="utf-8")
    mark_box = _viewbox(mark)
    inlined = (
        f'<svg x="18" y="10" width="200" height="200" viewBox="{mark_box}">'
        f"{_inner(mark)}</svg>"
    )
    logo_inner = re.sub(
        r'<image\b[^>]*\bhref="mark\.svg"[^>]*/>',
        inlined,
        _inner(logo),
        count=1,
    )
    if "mark.svg" in logo_inner:
        raise SystemExit("logo.svg still points at mark.svg; the card would be blank")
    logo_inner = re.sub(
        r'font-family="[^"]*"',
        f'font-family="{_FONT}"',
        logo_inner,
        count=1,
    )
    logo_box = _viewbox(logo)
    parts = [float(part) for part in logo_box.split()]
    box_w, box_h = int(parts[2]), int(parts[3])
    native = tmp / "logo.png"
    native_svg = tmp / "logo.svg"
    native_svg.write_text(
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{box_w}" height="{box_h}" '
        f'viewBox="{logo_box}">{logo_inner}</svg>\n',
        encoding="utf-8",
    )
    subprocess.run(
        [rsvg, "-w", str(box_w), "-h", str(box_h), str(native_svg), "-o", str(native)],
        check=True,
    )
    left, top, right, bottom = _ink_box(native)
    # A few pixels of the logo's own ground, so antialiased edges are not cropped.
    pad = 8
    left = max(0, left - pad)
    top = max(0, top - pad)
    right = min(box_w - 1, right + pad)
    bottom = min(box_h - 1, bottom + pad)
    crop_w = right - left + 1
    crop_h = bottom - top + 1
    width, height = _CANVAS
    scale = min((width - 2 * _PAD) / crop_w, (height - 2 * _PAD) / crop_h)
    drawn_w, drawn_h = crop_w * scale, crop_h * scale
    x = (width - drawn_w) / 2
    y = (height - drawn_h) / 2
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
        f'viewBox="0 0 {width} {height}">'
        f'<rect width="{width}" height="{height}" fill="{_BG}"/>'
        f'<svg x="{x:.2f}" y="{y:.2f}" width="{drawn_w:.2f}" height="{drawn_h:.2f}" '
        f'viewBox="{left} {top} {crop_w} {crop_h}">{logo_inner}</svg>'
        f"</svg>\n"
    )


def main() -> int:
    rsvg = shutil.which("rsvg-convert")
    if not rsvg:
        print("render_og_image: need rsvg-convert (librsvg)", file=sys.stderr)
        return 1
    if not _LOGO.is_file() or not _MARK.is_file():
        print("render_og_image: logo.svg or mark.svg is missing", file=sys.stderr)
        return 1
    with tempfile.TemporaryDirectory() as tmp:
        tmp_path = Path(tmp)
        svg = tmp_path / "og.svg"
        svg.write_text(_card_svg(rsvg, tmp_path), encoding="utf-8")
        subprocess.run([rsvg, str(svg), "-o", str(_OUT)], check=True)
    data = _OUT.read_bytes()
    if data[16:24] != (1200).to_bytes(4, "big") + (630).to_bytes(4, "big"):
        print(f"render_og_image: {_OUT} is not 1200x630", file=sys.stderr)
        return 1
    print(f"render_og_image: wrote {_OUT} ({_OUT.stat().st_size} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
