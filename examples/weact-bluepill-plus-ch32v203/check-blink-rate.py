#!/usr/bin/env python3
"""Check the PB2 blink rate of template-smoke.yaml from result.json.

The template toggles PB2 every 250 passes of a 1 ms loop until KEY (PA0) is
pressed and released, then every 100 passes. At the 8 MHz HSI clock, each
half-period must be 250 ms (2,000,000 cycles) before the press and 100 ms
(800,000 cycles) after the release, within 1 % (the loop body adds about
34 cycles per pass).

usage: check-blink-rate.py <output-dir>/result.json
"""
import json
import sys

CPU_HZ = 8_000_000
PRESS_CYCLE = 7_000_000
RELEASE_CYCLE = 7_800_000
TOLERANCE = 0.01


def main() -> int:
    with open(sys.argv[1], encoding="utf-8") as f:
        result = json.load(f)
    channels = [
        c for c in result["logic_edges"]["channels"]
        if c["peripheral"] == "gpiob" and c["pin"] == 2
    ]
    if len(channels) != 1:
        print("FAIL: no gpiob:2 channel in logic_edges")
        return 1
    edges = [t["cycle"] for t in channels[0]["transitions"]]
    before = [b - a for a, b in zip(edges, edges[1:]) if b < PRESS_CYCLE]
    after = [b - a for a, b in zip(edges, edges[1:]) if a > RELEASE_CYCLE]

    ok = True
    for label, spans, want_ms, need in (
        ("before press", before, 250, 2),
        ("after release", after, 100, 5),
    ):
        want = want_ms * CPU_HZ // 1000
        bad = [s for s in spans if abs(s - want) > want * TOLERANCE]
        ms = ", ".join(f"{s * 1000 / CPU_HZ:.2f}" for s in spans)
        status = "ok" if len(spans) >= need and not bad else "FAIL"
        print(f"{status}: {label}: {len(spans)} half-periods [{ms}] ms, want {want_ms} ms +-1 %, need >= {need}")
        ok = ok and status == "ok"
    print("PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
