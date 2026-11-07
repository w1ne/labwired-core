#!/usr/bin/env python3
"""Check the SSD1306 frames LabWired captured from ch32fun's spi_oled against
the example's own arithmetic, recomputed here.

spi_oled.c (128x64 build) fills the 1024-byte frame buffer from rand.h's
LFSR (state 1 at reset, never reseeded; each rand() shifts 31 bits through
the 0x80200003 Galois taps), refreshes, then runs Conway's Life on the
buffer, a full refresh after each generation. conway() writes column x-1
only after column x has been computed from it, so a generation is a plain
synchronous step; cells past the panel edge are dead.

Every completed frame the panel latches is listed in the framebuffer
artifact's `frames_seen` with the SHA-256 of its 1024 GDDRAM bytes. This
script requires that list to be, in order: the blank frame ssd1306_init()
refreshes, the random fill, then Life generations 1, 2, 3, ... with none
skipped, repeated or wrong. It then requires the live GDDRAM bytes (which the
run may stop part-way through a refresh) to be generation N+1's first n
bytes followed by generation N's remainder, where N is the last listed frame.

    check-spi-oled.py result.json [--min-generations K]

result.json must be written with --inspect-bytes-max 4096 so the artifact
carries the GDDRAM bytes.
"""
import argparse
import hashlib
import json
import sys

W, H = 128, 64
PAGES = H // 8


def lfsr_bytes(count):
    state = 1
    out = []
    for _ in range(count):
        for _ in range(31):
            lsb = state & 1
            state >>= 1
            if lsb:
                state ^= 0x80200003
        out.append(state & 0x7FFFFFFF & 0xFF)
    return out


def pix(buf, x, y):
    if x < 0 or x >= W or y < 0 or y >= H:
        return 0
    return (buf[x + W * (y >> 3)] >> (y & 7)) & 1


def life(buf):
    out = [0] * len(buf)
    for x in range(W):
        for page in range(PAGES):
            d = 0
            for b in range(8):
                y = page * 8 + b
                n = sum(
                    pix(buf, x + dx, y + dy)
                    for dx in (-1, 0, 1)
                    for dy in (-1, 0, 1)
                    if dx or dy
                )
                if n == 3 or (n == 2 and pix(buf, x, y)):
                    d |= 1 << b
            out[x + W * page] = d
    return out


def sha(buf):
    return hashlib.sha256(bytes(buf)).hexdigest()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("result")
    ap.add_argument("--min-generations", type=int, default=1)
    a = ap.parse_args()

    result = json.load(open(a.result))
    art = next(
        d["artifacts"][0] for d in result["inspect"]["devices"] if d["id"] == "oled"
    )
    seen = art["meta"]["frames_seen"]
    live = art.get("bytes")
    if live is None or len(live) != W * PAGES:
        sys.exit("FAIL: no 1024-byte GDDRAM payload (run with --inspect-bytes-max 4096)")

    frames = [[0] * (W * PAGES), lfsr_bytes(W * PAGES)]
    labels = ["blank (ssd1306_init refresh)", "random fill"]
    while len(frames) < len(seen) + 1:
        frames.append(life(frames[-1]))
        labels.append("Life generation %d" % (len(frames) - 2))

    for i, entry in enumerate(seen):
        if entry["sha256"] != sha(frames[i]):
            sys.exit(
                "FAIL: frame %d (t=%d us) is not the %s" % (i, entry["t_us"], labels[i])
            )
        print("ok  frame %2d  t=%7d us  %s" % (i, entry["t_us"], labels[i]))

    last = len(seen) - 1
    cur, nxt = frames[last], frames[last + 1]
    n = 0
    while n < len(live) and live[n] == nxt[n]:
        n += 1
    if live[n:] != cur[n:]:
        sys.exit(
            "FAIL: live GDDRAM is neither the %s nor a partial refresh into the next frame"
            % labels[last]
        )
    print("ok  live GDDRAM = %s, first %d bytes already %s" % (labels[last], n, labels[last + 1]))

    gens = last - 1
    if gens < a.min_generations:
        sys.exit("FAIL: only %d Life generations, wanted %d" % (gens, a.min_generations))
    print("PASS: %d frames, %d Life generations, every frame exact" % (len(seen), gens))


if __name__ == "__main__":
    main()
