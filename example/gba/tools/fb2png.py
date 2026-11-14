"""Convert a raw RGB565 240x160 framebuffer dump to a scaled PNG.

Uses only the standard library (zlib), so it works without PIL.
Usage: py fb2png.py <in.bin> <out.png> [scale]
"""

import pathlib
import struct
import sys
import zlib

W, H = 240, 160


def chunk(tag: bytes, payload: bytes) -> bytes:
    return (
        struct.pack(">I", len(payload))
        + tag
        + payload
        + struct.pack(">I", zlib.crc32(tag + payload) & 0xFFFFFFFF)
    )


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    src = pathlib.Path(sys.argv[1])
    dst = pathlib.Path(sys.argv[2])
    scale = int(sys.argv[3]) if len(sys.argv) > 3 else 4

    data = src.read_bytes()
    if len(data) < W * H * 2:
        print(f"error: {src} holds {len(data)} bytes, need {W * H * 2}")
        return 1
    pixels = struct.unpack("<%dH" % (W * H), data[: W * H * 2])
    print(
        f"pixels={len(pixels)} nonzero={sum(1 for v in pixels if v)} "
        f"unique={len(set(pixels))}"
    )

    rows = []
    for y in range(H * scale):
        base = (y // scale) * W
        line = bytearray()
        for x in range(W * scale):
            value = pixels[base + x // scale]
            line += bytes(
                (
                    ((value >> 11) & 0x1F) * 255 // 31,
                    ((value >> 5) & 0x3F) * 255 // 63,
                    (value & 0x1F) * 255 // 31,
                )
            )
        rows.append(b"\x00" + bytes(line))

    header = struct.pack(">IIBBBBB", W * scale, H * scale, 8, 2, 0, 0, 0)
    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(b"".join(rows), 9))
        + chunk(b"IEND", b"")
    )
    dst.write_bytes(png)
    print(f"wrote {dst} ({dst.stat().st_size} bytes, {W * scale}x{H * scale})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
