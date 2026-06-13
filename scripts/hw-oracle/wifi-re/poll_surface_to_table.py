#!/usr/bin/env python3
"""Reduce a trace_poll.sh log to a candidate MAC poll-bit table.

Input:  poll_trace.log — repeated "##HIT n" blocks, each followed by a `reg pc`
        capture and an `mdw <base> <count>` dump of the MAC window, ending at
        a "##BRACKET done" marker.

Output: one row per MAC offset whose value CHANGED across the captured reads,
        with the deduped value sequence and any bit that transitioned 0->1 (the
        candidate "ready/done" bit the driver busy-waits on). The offset whose
        bit rose just before "##BRACKET done" is the prime suspect for the bit
        the behavioral model must flip to release esp_wifi_start().

Usage:  python3 poll_surface_to_table.py poll_trace.log

This recovers semantics from a real silicon trace — it does not invent them.
Hand the printed table to crates/core/src/peripherals/esp32c3/wifi_mac.rs
(POLL_TABLE) so the model releases on the real value the driver expects.
"""
import re
import sys

MDW = re.compile(r"^(0x[0-9a-fA-F]+):\s+((?:[0-9a-fA-F]{8}\s*)+)")


def main(argv):
    if len(argv) != 2:
        print("usage: poll_surface_to_table.py <poll_trace.log>", file=sys.stderr)
        return 2
    with open(argv[1], "r", errors="replace") as fh:
        lines = fh.readlines()

    # offset -> deduped sequence of observed values (consecutive repeats collapsed)
    seq: dict[int, list[int]] = {}
    order: list[int] = []
    # offset -> mask of bits that rose (0->1) on the most recent change
    last_rose: dict[int, int] = {}
    bracket_done_seen = False
    rose_before_done: dict[int, int] = {}

    for ln in lines:
        if "##BRACKET done" in ln:
            bracket_done_seen = True
            continue
        m = MDW.match(ln.strip())
        if not m:
            continue
        base = int(m.group(1), 16)
        words = m.group(2).split()
        for i, w in enumerate(words):
            off = base + i * 4
            v = int(w, 16)
            if off not in seq:
                seq[off] = []
                order.append(off)
            s = seq[off]
            if not s or s[-1] != v:
                if s:
                    rose = v & ~s[-1] & 0xFFFFFFFF
                    if rose:
                        last_rose[off] = rose
                        if not bracket_done_seen:
                            rose_before_done[off] = rose
                s.append(v)

    rows = [off for off in order if len(seq[off]) > 1]
    print(f"{'OFFSET':<12}  {'RDYBIT':<6}  VALUE SEQUENCE (deduped)")
    print(f"{'------':<12}  {'------':<6}  ------------------------")
    if not rows:
        print("(no changing offsets — 0 hits, or the poll window is elsewhere; "
              "widen the watchpoint span in trace_poll.sh)")
        return 0
    for off in rows:
        rdy = ""
        if off in last_rose:
            rdy = "b" + str((last_rose[off] & -last_rose[off]).bit_length() - 1)
        vals = " -> ".join(f"0x{v:08x}" for v in seq[off])
        print(f"0x{off:08x}  {rdy:<6}  {vals}")

    # Prime suspect: the last bit that rose before the bracket closed.
    if rose_before_done:
        suspect = max(rose_before_done)  # later offset = later in the rising sweep
        bit = (rose_before_done[suspect] & -rose_before_done[suspect]).bit_length() - 1
        print(f"\nPrime release suspect: 0x{suspect:08x} bit b{bit} "
              "(rose just before esp_wifi_start returned).")
    print(f"\n{len(rows)} candidate poll register(s).")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
