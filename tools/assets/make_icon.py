"""The app icon: an oxide wafer cross-section, drawn with signed distances.

Offline only: writes crates/oxide-ui/icon.png (1024×1024), which nsh package turns into
each platform's icon. Deterministic, no dependencies beyond the standard library.
"""

import math
import os
import struct
import zlib

SIZE = 1024
BG = (0x0B, 0x10, 0x16)
TEAL = (0x2D, 0xD4, 0xBF)
AMBER = (0xFB, 0xBF, 0x24)
GOOD = (0x34, 0xD3, 0x99)
INK = (0x07, 0x0C, 0x13)


def rounded(x, y, half, radius):
    qx = abs(x) - (half - radius)
    qy = abs(y) - (half - radius)
    outside = math.hypot(max(qx, 0.0), max(qy, 0.0))
    inside = min(max(qx, qy), 0.0)
    return outside + inside - radius


def ring(d, radius, width):
    return abs(d - radius) - width


def coverage(distance):
    """A filled shape: the signed distance is negative inside."""
    return max(0.0, min(1.0, 0.5 - distance * 1.2))


def line_alpha(distance, width=0.010):
    """A thin line: `distance` is the distance to its centre line."""
    return max(0.0, min(1.0, 1.0 - abs(distance) / width))


def blend(dst, src, alpha):
    return tuple(int(round(dst[i] * (1 - alpha) + src[i] * alpha)) for i in range(3))


pixels = bytearray()
for py in range(SIZE):
    row = bytearray()
    for px in range(SIZE):
        x = (px + 0.5) / SIZE * 2 - 1
        y = (py + 0.5) / SIZE * 2 - 1
        color = (0, 0, 0)
        # The rounded tile.
        tile = rounded(x, y, 0.92, 0.22)
        a = coverage(tile)
        if a <= 0:
            row.extend((0, 0, 0, 0))
            continue
        color = blend((0, 0, 0), BG, a)
        # A faint floor grid behind the wafer.
        grid = min(abs(((x * 6) % 1) - 0.5), abs(((y * 6) % 1) - 0.5))
        if tile < -0.02:
            color = blend(color, (0x1B, 0x26, 0x34), line_alpha(grid, 0.006) * a)
        # The wafer: three concentric rings and a filled disc.
        d = math.hypot(x, y * 1.0)
        if d < 0.62:
            color = blend(color, INK, coverage(d - 0.62) * a)
            for radius in (0.56, 0.40, 0.24):
                alpha = line_alpha(ring(d, radius, 0.012), 0.014) * a
                if alpha > 0:
                    color = blend(color, TEAL, alpha)
            # The notch: a wedge at the bottom, amber.
            angle = math.atan2(y, x)
            if (
                d > 0.52
                and -2.05 < angle < -1.09
                and tile < -0.03
            ):
                color = blend(color, AMBER, 0.95 * a)
        # One green die: the line is running.
        die = math.hypot(x - 0.30, y + 0.30)
        if die < 0.075:
            color = blend(color, GOOD, coverage(die - 0.075) * a)
        # A dark rim so the tile reads on light desktops.
        if -0.03 < tile < 0.0:
            color = blend(color, INK, 0.9)
        row.extend((color[0], color[1], color[2], 255))
    pixels.extend(row)


def chunk(tag, data):
    return (
        struct.pack(">I", len(data))
        + tag
        + data
        + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
    )


raw = b"".join(b"\x00" + bytes(pixels[y * SIZE * 4 : (y + 1) * SIZE * 4]) for y in range(SIZE))
png = (
    b"\x89PNG\r\n\x1a\n"
    + chunk(b"IHDR", struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0))
    + chunk(b"IDAT", zlib.compress(raw, 9))
    + chunk(b"IEND", b"")
)
root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
path = os.path.join(root, "crates", "oxide-ui", "icon.png")
with open(path, "wb") as handle:
    handle.write(png)
print("wrote", path, len(png), "bytes")
