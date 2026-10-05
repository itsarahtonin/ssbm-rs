"""Draws the player's icon, a hexagonal ring in the style of the reflector move (original
artwork, nothing taken from the game), and writes it as ssbm-rs.ico (16 to 256 pixels, for
the Windows executable), ssbm-rs.png (256 pixels) and window.rgba (64 by 64 RGBA pixels, the
window's icon). The band shades from a periwinkle edge to a pale center; small sizes get a
wider band so the hole stays open. With --iconset DIR it writes the PNGs macOS's iconutil makes
an .icns from instead (16 to 1024 pixels), for the release workflow's app bundle.

    python tools/run/icon/make.py [--iconset DIR]
"""

import math
import os
import struct
import sys
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
SIZES = [16, 20, 24, 32, 40, 48, 64, 128, 256]
EDGE = (111, 143, 238)
MIDDLE = (207, 230, 253)
ROOT3 = math.sqrt(3)


def hex_distance(x, y):
    """A point-up hexagon's distance function: its apothem where the point lies on one."""
    return max(abs(x), abs(x / 2 + y * ROOT3 / 2), abs(x / 2 - y * ROOT3 / 2))


def render(size):
    """RGBA rows of the icon at `size` pixels, supersampled."""
    hole = 0.36 if size <= 24 else 0.48
    radius = size / 2 * 0.97
    outer = radius * ROOT3 / 2
    inner = outer * hole
    samples = 8 if size <= 64 else 4 if size <= 256 else 2
    rows = []
    for py in range(size):
        row = bytearray()
        for px in range(size):
            acc = [0.0, 0.0, 0.0]
            cover = 0
            for sy in range(samples):
                for sx in range(samples):
                    x = px + (sx + 0.5) / samples - size / 2
                    y = py + (sy + 0.5) / samples - size / 2
                    d = hex_distance(x, y)
                    if inner <= d <= outer:
                        t = (d - inner) / (outer - inner)
                        s = 1 - abs(2 * t - 1) ** 1.6
                        for i in range(3):
                            acc[i] += EDGE[i] + (MIDDLE[i] - EDGE[i]) * s
                        cover += 1
            if cover:
                row += bytes(round(c / cover) for c in acc)
                row.append(round(255 * cover / samples**2))
            else:
                row += b"\0\0\0\0"
        rows.append(bytes(row))
    return rows


def png(size, rows):
    def chunk(kind, data):
        return (struct.pack(">I", len(data)) + kind + data
                + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF))
    raw = b"".join(b"\0" + row for row in rows)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


def ico(images):
    """An ICO holding each (size, PNG) image."""
    head = struct.pack("<HHH", 0, 1, len(images))
    offset = len(head) + 16 * len(images)
    entries, data = b"", b""
    for size, image in images:
        side = 0 if size == 256 else size
        entries += struct.pack("<BBBBHHII", side, side, 0, 0, 1, 32, len(image), offset + len(data))
        data += image
    return head + entries + data


def iconset(folder):
    os.makedirs(folder, exist_ok=True)
    for size in [16, 32, 128, 256, 512]:
        for scale, suffix in [(1, ""), (2, "@2x")]:
            side = size * scale
            with open(os.path.join(folder, f"icon_{size}x{size}{suffix}.png"), "wb") as f:
                f.write(png(side, render(side)))


def main():
    if sys.argv[1:2] == ["--iconset"]:
        iconset(sys.argv[2])
        return
    images = []
    for size in SIZES:
        rows = render(size)
        images.append((size, png(size, rows)))
        if size == 64:
            with open(os.path.join(HERE, "window.rgba"), "wb") as f:
                f.write(b"".join(rows))
    with open(os.path.join(HERE, "ssbm-rs.ico"), "wb") as f:
        f.write(ico(images))
    with open(os.path.join(HERE, "ssbm-rs.png"), "wb") as f:
        f.write(images[-1][1])


if __name__ == "__main__":
    main()
