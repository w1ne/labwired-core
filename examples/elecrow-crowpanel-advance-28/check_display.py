#!/usr/bin/env python3
"""Check exact RGB565 colors after conversion to PPM by LabWired CLI."""
import collections
import pathlib
import sys


def read_ppm(path):
    with path.open("rb") as f:
        assert f.readline().strip() == b"P6", "expected binary PPM"
        header = f.readline()
        while header.startswith(b"#"):
            header = f.readline()
        width, height = map(int, header.split())
        assert f.readline().strip() == b"255", "expected 8-bit RGB"
        rgb = f.read()
    assert len(rgb) == width * height * 3, "incomplete image"
    return width, height, collections.Counter(zip(rgb[0::3], rgb[1::3], rgb[2::3]))


def main():
    mode, filename = sys.argv[1:]
    width, height, colors = read_ppm(pathlib.Path(filename))
    if mode == "positive":
        assert (width, height) == (320, 240), (width, height)
        for rgb in [(255, 0, 0), (0, 255, 0), (0, 0, 255)]:
            assert colors[rgb] == 48 * 48, (rgb, colors[rgb])
        assert colors[(0, 0, 0)] == width * height - 3 * 48 * 48
    elif mode == "negative":
        assert width * height == 320 * 240
        assert len(colors) == 1 and colors[(0, 0, 0)] == width * height
    else:
        raise ValueError(mode)
    print(f"PASS {mode}: {width}x{height}, colors={dict(colors)}")


if __name__ == "__main__":
    main()
