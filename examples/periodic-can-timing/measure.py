#!/usr/bin/env python3
"""Measure the periodic CAN frame schedule from a can-bus recording.

Reads `can-bus.jsonl` (written by `labwired test --output-dir`) and reports
the inter-frame period of the engine's 10 ms RPM frame (id 0x100) in virtual
time: cycles (exact) and microseconds (derived from the recording's clock_hz).
Every interval must carry the same cycle count — the point of the demo is
that the scheduler is cycle-exact, with zero jitter.

Usage: python3 measure.py <can-bus.jsonl> [intervals]
"""

import json
import sys
from collections import Counter


def main() -> int:
    path = sys.argv[1] if len(sys.argv) > 1 else "out/can-bus.jsonl"
    n = int(sys.argv[2]) if len(sys.argv) > 2 else 100

    frames = []
    with open(path) as fh:
        for line in fh:
            rec = json.loads(line)
            if "clock_hz" in rec:
                continue
            if rec.get("id") == 0x100 and rec.get("dir") == "tx":
                frames.append(rec)

    if len(frames) < n + 1:
        print(f"need at least {n + 1} RPM frames, recording has {len(frames)}")
        return 1

    hz = 72_000_000  # STM32F103 core clock; can-bus.jsonl's header also carries it
    with open(path) as fh:
        header = json.loads(fh.readline())
        hz = header.get("clock_hz", hz)

    cycles = [f["cycle"] for f in frames]
    deltas = [b - a for a, b in zip(cycles, cycles[1:])]
    counts = Counter(deltas)

    print(f"frames: {len(frames)}  (id 0x100, tx)")
    print(f"inter-frame deltas: {dict(counts)}")
    if len(counts) != 1:
        print("FAIL: the schedule jitters — more than one distinct delta")
        return 1

    delta = deltas[0]
    span = cycles[n] - cycles[0]
    print(f"period: {delta} cycles = {delta / hz * 1e3:.6f} ms (constant to the cycle)")
    print(f"{n} intervals: {span} cycles = {span / hz:.9f} s virtual")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
