#!/usr/bin/env python3
"""Replay two bridge transcripts of a dual-bus run in virtual-time order.

usage: irq-replay.py TRANSCRIPT_DIR or|last

Reports:
  overlap  transfers of the first bus whose START..DONE interval overlaps a
           transfer of the second bus (both buses active on the shared SPI)
  latency  for each bridge, rise of its IRQ level -> its next register access
           (the ISR reading S), median and max
  hidden   episodes where a bridge holds its level at 1 but the GIC input,
           as QEMU drives it, is 0: `or` = OR of all levels (irq-or=on),
           `last` = last writer wins (irq-or=off). Longest episode in ns.
"""

import os
import sys

d, mode = sys.argv[1], sys.argv[2]
names = sorted(f[: -len(".transcript")] for f in os.listdir(d) if f.endswith(".transcript"))
rows = {n: [l.split() for l in open(os.path.join(d, n + ".transcript"))] for n in names}


def transfers(rs):
    out, start = [], None
    for r in rs:
        if r[1] == "WRITE" and r[2] == "0x00" and int(r[3], 16) & 0x80 and start is None:
            start = int(r[0])
        if start is not None and r[1] == "SYNC" and r[4] == "1":
            out.append((start, int(r[0])))
            start = None
    return out


a, b = (transfers(rows[n]) for n in names[:2])
overlap = sum(1 for s, e in a if any(s2 < e and s < e2 for s2, e2 in b))
print(f"overlap: {overlap} of {len(a)} {names[0]} transfers overlap a {names[1]} transfer")

for n in names:
    lat, prev = [], 0
    rs = rows[n]
    for k, r in enumerate(rs):
        if int(r[4]) and not prev and k + 1 < len(rs):
            lat.append(int(rs[k + 1][0]) - int(r[0]))
        prev = int(r[4])
    lat.sort()
    print(f"latency {n}: {len(lat)} rises, median {lat[len(lat) // 2]} ns, max {lat[-1]} ns")

ev = sorted((int(r[0]), n, k, int(r[4])) for n in names for k, r in enumerate(rows[n]))
lvl = dict.fromkeys(names, 0)
since, episodes, worst = {}, 0, 0
for t, n, _, irq in ev:
    lvl[n] = irq
    line = any(lvl.values()) if mode == "or" else bool(irq)
    for x in names:
        if lvl[x] and not line:
            if x not in since:
                since[x] = t
                episodes += 1
        elif x in since:
            worst = max(worst, t - since.pop(x))
print(f"hidden ({mode}): {episodes} episodes, longest {worst} ns")
