#!/usr/bin/env python3
"""Reduce a trace_poll.sh log to a candidate MAC poll-bit table.

Input:  poll_trace.log — a "##DONE_ADDR <pc>" line, then repeated "##HIT n"
        blocks each followed by a `reg pc` capture and an `mdw <base> <count>`
        dump of the MAC window. The watchpoint halts the core on every load
        against the window; trace_poll.sh ends the capture when the driver
        stops touching it (##HALT_TIMEOUT) or after MAX_HITS.

Output:
  1. POLL table — offsets whose value changed across the reads, the bit that
     rose 0->1 last *before esp_wifi_start returned* (pc reached DONE_ADDR), and
     the deduped value sequence. The release bit lives here.
  2. SPIN table — which PC read which offset and how often. A PC that reads one
     offset many times is a busy-wait; that offset is the status register and
     its rising bit is what the model must flip to release the spin.

This recovers semantics from a real silicon trace — it does not invent them.
Feed the result to crates/core/src/peripherals/esp32c3/wifi_mac.rs (POLL_TABLE).
"""
import re
import sys

MDW = re.compile(r"^(0x[0-9a-fA-F]+):\s+((?:[0-9a-fA-F]{8}\s*)+)")
PC = re.compile(r"pc \(/32\):\s*(0x[0-9a-fA-F]+)")
DONE = re.compile(r"##DONE_ADDR\s+(0x[0-9a-fA-F]+)")
HIT = re.compile(r"##HIT\s+(\d+)")


def lsb_index(mask):
    return (mask & -mask).bit_length() - 1


def main(argv):
    if len(argv) != 2:
        print("usage: poll_surface_to_table.py <poll_trace.log>", file=sys.stderr)
        return 2
    with open(argv[1], "r", errors="replace") as fh:
        lines = fh.readlines()

    done_addr = None
    seq, order = {}, []          # offset -> deduped value list
    last_rose, rose_before_done = {}, {}
    spin = {}                    # (pc, offset) -> count
    cur_pc = None
    bracket_done_seen = False
    done_hit = None
    hit_idx = 0

    for ln in lines:
        s = ln.strip()
        m = DONE.search(s)
        if m and done_addr is None:
            done_addr = int(m.group(1), 16)
            continue
        m = HIT.search(s)
        if m:
            hit_idx = int(m.group(1))
            cur_pc = None
            continue
        m = PC.search(s)
        if m:
            cur_pc = int(m.group(1), 16)
            if done_addr is not None and cur_pc == done_addr and not bracket_done_seen:
                bracket_done_seen = True
                done_hit = hit_idx
            continue
        m = MDW.match(s)
        if not m:
            continue
        base = int(m.group(1), 16)
        for i, w in enumerate(m.group(2).split()):
            off = base + i * 4
            v = int(w, 16)
            if off not in seq:
                seq[off] = []
                order.append(off)
            vals = seq[off]
            if not vals or vals[-1] != v:
                if vals:
                    rose = v & ~vals[-1] & 0xFFFFFFFF
                    if rose:
                        last_rose[off] = rose
                        if not bracket_done_seen:
                            rose_before_done[off] = rose
                vals.append(v)
            if cur_pc is not None:
                spin[(cur_pc, off)] = spin.get((cur_pc, off), 0) + 1

    # ---- POLL table ----
    changed = [o for o in order if len(seq[o]) > 1]
    print(f"DONE_ADDR={'0x%08x' % done_addr if done_addr else '?'}  "
          f"bracket closed at HIT {done_hit if done_hit else '(not seen)'}\n")
    print(f"{'OFFSET':<12}  {'RDYBIT':<6}  VALUE SEQUENCE (deduped, truncated)")
    print(f"{'------':<12}  {'------':<6}  -----------------------------------")
    for o in changed:
        rdy = f"b{lsb_index(last_rose[o])}" if o in last_rose else ""
        vals = seq[o]
        shown = " -> ".join(f"{v:08x}" for v in vals[:8]) + (" …" if len(vals) > 8 else "")
        print(f"0x{o:08x}  {rdy:<6}  {shown}")

    # ---- SPIN table: PCs that hammer one offset are busy-waits ----
    print(f"\n{'SPIN PC':<12}  {'OFFSET':<12}  READS  (pc reading one reg repeatedly = busy-wait)")
    print(f"{'-------':<12}  {'------':<12}  -----")
    for (pc, off), n in sorted(spin.items(), key=lambda kv: -kv[1])[:12]:
        print(f"0x{pc:08x}  0x{off:08x}  {n}")

    # ---- release suspect: last bit to rise before the bracket closed ----
    if rose_before_done:
        suspect = max(rose_before_done, key=lambda o: order.index(o))
        print(f"\nPrime release suspect: 0x{suspect:08x} bit "
              f"b{lsb_index(rose_before_done[suspect])} "
              "(rose just before esp_wifi_start returned).")
    elif not changed:
        print("\n(no changing offsets — 0 hits or wrong window)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
