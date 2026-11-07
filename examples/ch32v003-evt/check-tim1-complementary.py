#!/usr/bin/env python3
"""Check the TIM1 complementary pair of ch32fun's tim1_pwm_complementary_outputs.

Reads the `logic_edges` block of a `labwired test` result.json captured with
`--watch-gpio gpioc:6 --watch-gpio gpioc:3` and checks, for every whole PWM
period in the captured window, what the unmodified example programs:

- AFIO_PCFR1.TIM1_RM = 01 puts CH1 on PC6 and CH1N on PC3 (a model that
  ignored the remap leaves both pads flat and this fails at once);
- PSC 9, ATRLR 10000 at HCLK (48 MHz): a period of 10001 counts x 10 =
  100,010 cycles, on both pads, every period;
- CH1CVR 5000, PWM mode 1: OC1REF is high for 50,000 cycles;
- CC1P|CC1NP make both pads active low, so with dead time DT (CK_INT
  cycles, CKD = 0):
    PC3 (CH1N) rises when OC1REF rises; PC6 (CH1) falls DT later;
    PC6 rises when OC1REF falls;       PC3 falls DT later;
  i.e. both pads sit high (both switches off) for DT on each edge and are
  never low together;
- DT follows BDTR.DTG through the CH32V003 RM / RM0008 encoding
  (0xx: DTG; 10x: (64 + DTG[5:0]) x 2; 110: (32 + DTG[4:0]) x 8;
  111: (32 + DTG[4:0]) x 16), and the example writes DTG = count & 255 every
  Delay_Ms(20). So the decoded DTG only ever steps by +1 (mod 256), each
  value is held for about 960,000 cycles (9.6 periods), and a run that is
  long enough passes through all 256 values and the wrap to 0.

The input may be gzipped (the committed capture is pwm-capture.json.gz).
Prints a per-encoding summary and exits non-zero on the first mismatch.
"""
import gzip
import json
import sys

PERIOD = 10001 * 10
HIGH = 5000 * 10
HOLD = 48_000_000 * 20 // 1000  # cycles per Delay_Ms(20)


def dead_time(dtg):
    if dtg < 0x80:
        return dtg
    if dtg < 0xC0:
        return (64 + (dtg & 0x3F)) * 2
    if dtg < 0xE0:
        return (32 + (dtg & 0x1F)) * 8
    return (32 + (dtg & 0x1F)) * 16


DTG_OF = {dead_time(d): d for d in range(256)}
assert len(DTG_OF) == 256, "the DTG encoding is one-to-one"


def edges(chan, value):
    return [e["cycle"] for e in chan["transitions"] if e["value"] == value]


def fail(msg):
    print(f"FAIL: {msg}")
    sys.exit(1)


def main(path, require_full_sweep):
    opener = gzip.open if path.endswith(".gz") else open
    with opener(path, "rt") as f:
        logic = json.load(f)["logic_edges"]
    if logic.get("dropped"):
        fail(f"capture dropped {logic['dropped']} edges")
    chans = {(c["peripheral"], c["pin"]): c for c in logic["channels"]}
    ch1, ch1n = chans[("gpioc", 6)], chans[("gpioc", 3)]
    for c, want in ((ch1, "TIM1_CH1"), (ch1n, "TIM1_CH1N")):
        if c.get("func") != want:
            fail(f"PC{c['pin']} reports {c.get('func')!r}, expected {want}")
    n_rise, n_fall = edges(ch1n, 1), edges(ch1n, 0)
    c_rise, c_fall = edges(ch1, 1), edges(ch1, 0)
    if len(n_rise) < 3:
        fail("fewer than three CH1N rising edges: TIM1 never reached PC3")

    # Each period starts at a CH1N rising edge (OC1REF rising, CNT = 0).
    # Skip the first, which can sit in the start-up of the outputs.
    periods = list(zip(n_rise[1:], n_rise[2:]))
    gaps = []  # (cycle, dead time) in edge order
    ci = fi = ri = 0
    for start, end in periods:
        if end - start != PERIOD:
            fail(f"CH1N period {end - start} at cycle {start}, expected {PERIOD}")
        while ci < len(c_fall) and c_fall[ci] < start:
            ci += 1
        while ri < len(c_rise) and c_rise[ri] < start:
            ri += 1
        while fi < len(n_fall) and n_fall[fi] < start:
            fi += 1
        if ci >= len(c_fall) or ri >= len(c_rise) or fi >= len(n_fall):
            fail(f"period at {start} is missing an edge")
        cf, cr, nf = c_fall[ci], c_rise[ri], n_fall[fi]
        # Order inside one period: CH1N up, CH1 down (DT later), CH1 up
        # (OC1REF falls), CH1N down (DT later), next CH1N up.
        if not start <= cf < cr <= nf < end:
            fail(f"edge order broken in period at {start}: "
                 f"CH1 fall {cf}, CH1 rise {cr}, CH1N fall {nf}")
        if cr - start != HIGH:
            fail(f"OC1REF high for {cr - start} cycles at {start}, expected {HIGH}")
        for at, dt in ((start, cf - start), (cr, nf - cr)):
            if dt not in DTG_OF:
                fail(f"dead time {dt} at cycle {at} is not a DTG encoding")
            gaps.append((at, DTG_OF[dt]))

    # DTG walks +1 (mod 256), each value held for Delay_Ms(20).
    runs = []  # (dtg, first cycle, gap count)
    for at, dtg in gaps:
        if runs and runs[-1][0] == dtg:
            runs[-1][2] += 1
            continue
        if runs and dtg != (runs[-1][0] + 1) & 0xFF:
            fail(f"DTG jumped {runs[-1][0]} -> {dtg} at cycle {at}")
        runs.append([dtg, at, 1])
    # Whole runs only: the first and last are cut by the capture window.
    holds = [b[1] - a[1] for a, b in zip(runs[1:], runs[2:])]
    for (dtg, at, _), hold in zip(runs[1:], holds):
        # A step lands on the next edge after the write: the hold is
        # Delay_Ms(20) plus the loop body, quantised to the half-period
        # spacing of the gaps.
        if not HOLD - PERIOD // 2 <= hold <= HOLD + PERIOD:
            fail(f"DTG {dtg} held {hold} cycles from {at}, expected ~{HOLD}")

    print(f"window: cycles {periods[0][0]}..{periods[-1][1]} "
          f"({len(periods)} whole periods of {PERIOD} cycles, "
          f"OC1REF high {HIGH})")
    print(f"dead-time gaps: {len(gaps)} (two per period), "
          f"{len(runs)} DTG values, mean hold {sum(holds) / max(len(holds), 1):.0f} cycles")
    bands = [(0x00, 0x7F, "0xx: DT = DTG"),
             (0x80, 0xBF, "10x: (64+DTG[5:0])x2"),
             (0xC0, 0xDF, "110: (32+DTG[4:0])x8"),
             (0xE0, 0xFF, "111: (32+DTG[4:0])x16")]
    seen = {r[0] for r in runs}
    for lo, hi, name in bands:
        got = sorted(d for d in seen if lo <= d <= hi)
        span = f"{got[0]}..{got[-1]}, DT {dead_time(got[0])}..{dead_time(got[-1])} cycles" if got else "-"
        print(f"  DTG {lo:3}..{hi:3} {name:24} {len(got):3}/{hi - lo + 1} values  {span}")
    wrapped = any(a[0] == 255 and b[0] == 0 for a, b in zip(runs, runs[1:]))
    print(f"  wrap 255 -> 0: {'yes' if wrapped else 'no'}")
    if require_full_sweep and (len(seen) != 256 or not wrapped):
        fail("--full: not every DTG value and the wrap were captured")
    print("PASS")


if __name__ == "__main__":
    args = [a for a in sys.argv[1:] if a != "--full"]
    if len(args) != 1:
        sys.exit("usage: check-tim1-complementary.py [--full] result.json[.gz]")
    main(args[0], "--full" in sys.argv[1:])
