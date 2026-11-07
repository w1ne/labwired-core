#!/usr/bin/env python3
"""Check the WS2812 colours LabWired decoded off PC6 against ch32fun's
ws2812bdemo, recomputed here from the example's own source.

The strip model decodes pulse widths on the PC6 pad (the SPI1 MOSI wire) and
latches a frame when the line rests low for 40 us. This script replays the
demo's colour math for frame N (N phase updates since boot) with the tables
read straight out of the example's color_utilities.h, and compares every LED.

    check-ws2812.py result.json [--frames N] [--color-utilities PATH]

result.json must be written with --inspect-bytes-max large enough to carry the
573-byte strip payload. Without --frames the script reports which frame the
payload matches.

Which LEDs reach the wire is not assumed either: `sent_leds` replays
ws2812b_dma_spi_led_driver.h's half-buffer protocol. With NR_LEDS = 191 the
half-transfer refill that writes LED 190 into the first half is the same call
that clears CFGR.CIRC, so the lap runs out after the second half and LED 190
is never clocked out. The driver sends 190 LEDs; so must the model.
"""
import argparse
import json
import os
import re
import sys

NR_LEDS = 191
CH32FUN_COMMIT = "b28d285460ce052a179dba8f4a22c6f73ff8e328"


def tables(path):
    src = open(path).read()
    out = {}
    for name in ("huetable", "rands", "sintable"):
        body = re.search(r"%s\[\]\s*=\s*\{(.*?)\};" % name, src, re.S).group(1)
        out[name] = [int(v, 16) for v in re.findall(r"0x[0-9a-fA-F]+", body)]
        assert len(out[name]) == 256, name
    return out


def sent_leds(n, reset=2, slots=8):
    """LED indices the driver clocks out for one WS2812BDMAStart(n), in
    order. DMALEDS 16, non-WSRAW: the 48-halfword buffer holds 8 LED slots,
    4 per half. WS2812FillBuffSec writes zeros for the `reset` leading slots
    and after the last LED; a first-half fill (the HT handler, tce = 1) that
    finds place == n clears CIRC, and the channel stops at the end of the
    second half it is already sending."""
    place, circ, buf = -reset, True, [None] * slots

    def fill(lo, hi, tce):
        nonlocal place, circ
        i = lo
        while place < 0 and i < hi:
            buf[i] = None
            i += 1
            place += 1
        while i < hi:
            if place >= n:
                buf[i:hi] = [None] * (hi - i)
                if tce:
                    if place == n:
                        circ = False
                    place += 1
                break
            buf[i] = place
            place += 1
            i += 1

    fill(0, slots, False)  # WS2812BDMAStart
    out, half = [], slots // 2
    while True:
        out += buf[:half]
        fill(0, half, True)  # HT: DMA now sends the second half
        out += buf[half:]
        if not circ:
            break  # TC with CIRC clear: CNTR stays 0
        fill(half, slots, False)  # TC: refill the second half
    return [x for x in out if x is not None]


def frame_wire_bytes(t, frames):
    """Wire bytes (GRB order as WSGRB clocks them) of the frame the demo
    sends after `frames` phase updates. tween stays at -NR_LEDS until
    frameno 1024, so every LED is the `fire` colour (TweenHexColors(.., 0))."""
    assert frames < 1024, "tween starts moving at frame 1024"
    hue, rands, sin = t["huetable"], t["rands"], t["sintable"]
    phases = [(k << 8) & 0xFFFF for k in range(NR_LEDS)]
    for _ in range(frames):
        for k in range(NR_LEDS):
            r = rands[k & 0xFF]
            phases[k] = (phases[k] + ((((r + 0xF) << 2) + ((r + 0xF) << 1)) >> 1)) & 0xFFFF
    out = []
    for k in sent_leds(NR_LEDS):
        rs = sin[phases[k] >> 8] >> 3
        fire = ((hue[(rs + 190) & 0xFF] >> 1) << 16) | hue[(rs + 30) & 0xFF] | (
            (hue[rs] >> 1) << 8
        )
        # WSGRB: bits 15..8, 7..0, then 23..16 go out first to last.
        out += [(fire >> 8) & 0xFF, fire & 0xFF, (fire >> 16) & 0xFF]
    return out


def strip_bytes(result):
    for dev in result["inspect"]["devices"]:
        if dev["id"] == "strip":
            art = dev["artifacts"][0]
            return art["meta"], art.get("bytes")
    sys.exit("no 'strip' device in result.json")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("result")
    ap.add_argument("--frames", type=int)
    ap.add_argument(
        "--color-utilities",
        default=os.path.join(
            os.environ.get("XDG_CACHE_HOME", os.path.expanduser("~/.cache")),
            "labwired",
            "ch32fun-" + CH32FUN_COMMIT,
            "examples/ws2812bdemo/color_utilities.h",
        ),
    )
    a = ap.parse_args()
    meta, got = strip_bytes(json.load(open(a.result)))
    print("strip meta:", json.dumps(meta, sort_keys=True))
    if got is None:
        sys.exit("FAIL: no payload bytes (run with --inspect-bytes-max 4096)")
    sent = len(sent_leds(NR_LEDS))
    if meta["pixels_decoded"] != sent:
        sys.exit("FAIL: %d LEDs decoded, the driver sends %d" % (meta["pixels_decoded"], sent))
    t = tables(a.color_utilities)
    candidates = [a.frames] if a.frames is not None else range(0, 64)
    for n in candidates:
        want = frame_wire_bytes(t, n)
        if got[: len(want)] == want:
            print(
                "PASS: all %d LEDs the driver sends (of NR_LEDS %d) match ws2812bdemo "
                "frame %d (%d phase updates)" % (sent, NR_LEDS, n, n)
            )
            return
    n = candidates[0]
    want = frame_wire_bytes(t, n)
    bad = [i // 3 for i in range(len(want)) if i >= len(got) or got[i] != want[i]]  # LED slot
    sys.exit("FAIL: frame %d mismatch at LEDs %s" % (n, sorted(set(bad))[:20]))


if __name__ == "__main__":
    main()
