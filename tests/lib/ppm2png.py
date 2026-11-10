#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Convert a QEMU screendump (PPM, P6) to PNG and say whether it is blank.

Usage: ppm2png.py <in.ppm> <out.png>
Prints "size WxH colors N dominant FRACTION" and exits 0 when the picture has
content (at least 8 colours and no single colour above 99.5 percent), 1 when
it looks blank, 2 on a bad file.
"""
import collections
import struct
import sys
import zlib


def main(src, dst):
    data = open(src, "rb").read()
    parts = data.split(None, 4)
    if len(parts) < 5 or parts[0] != b"P6" or parts[3] != b"255":
        print("not a P6 PPM", file=sys.stderr)
        return 2
    w, h = int(parts[1]), int(parts[2])
    pix = parts[4][-(w * h * 3):]
    if len(pix) != w * h * 3 or w == 0 or h == 0:
        print("truncated PPM", file=sys.stderr)
        return 2
    raw = b"".join(b"\x00" + pix[y * w * 3:(y + 1) * w * 3] for y in range(h))

    def chunk(kind, body):
        c = struct.pack(">I", len(body)) + kind + body
        return c + struct.pack(">I", zlib.crc32(kind + body))

    png = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
           + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b""))
    open(dst, "wb").write(png)

    count = collections.Counter(pix[i:i + 3] for i in range(0, len(pix), 3 * 7))
    total = sum(count.values())
    dominant = count.most_common(1)[0][1] / total
    print(f"size {w}x{h} colors {len(count)} dominant {dominant:.4f}")
    return 0 if len(count) >= 8 and dominant <= 0.995 else 1


if __name__ == "__main__":
    sys.exit(main(*sys.argv[1:3]))
