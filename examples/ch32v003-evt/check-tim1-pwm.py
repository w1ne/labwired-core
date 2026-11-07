#!/usr/bin/env python3
"""Check the TIM1 PWM waveforms of ch32fun's examples/tim1_pwm.

Reads the `logic_edges` block of a `labwired test` result.json captured with
`--watch-gpio gpiod:0 --watch-gpio gpioc:4` and checks, for every whole PWM
period in the captured window, what the unmodified example programs (TIM1,
the advanced-control timer, at HCLK, PSC 0, ATRLR 255, PWM mode 1, BDTR.MOE):

- the period is 256 timer counts (256 CPU cycles at 48 MHz, 187.5 kHz);
- CH1N (PD0) is LOW for `count` cycles per period. The example sets
  CC1NE|CC1NP and leaves CC1E clear, so the complementary pad carries OC1REF
  itself (CH32V003 RM / RM0008 Table 83: CCxNE alone gives OCxN = OCxREF),
  inverted by CC1NP. A model that drove !OC1REF there, or ignored CC1NE,
  fails here;
- CH4 (PC4, CC4E|CC4P) is LOW for `(count + 128) & 255` cycles;
- at a step, CH4 may lag CH1N by one period (CH1CVR is stored first), never
  longer and never ahead;
- `count` only ever steps by +1 (mod 256), each value held for Delay_Ms(5)
  worth of periods (240,000 cycles / 256 = 937.5).

The input may be gzipped (the committed capture is pwm-capture.json.gz).
Prints a per-duty summary and exits non-zero on the first mismatch.
"""
import gzip
import json
import sys

PERIOD = 256
HOLD = 48_000_000 * 5 // 1000 / PERIOD  # periods per Delay_Ms(5)


def low_time(transitions, initial, start, end):
    """Cycles the line spends at 0 inside [start, end)."""
    level = initial
    t = start
    low = 0
    for edge in transitions:
        c = edge["cycle"]
        if c <= start:
            level = edge["value"]
            continue
        if c >= end:
            break
        if level == 0:
            low += c - t
        t = c
        level = edge["value"]
    if level == 0:
        low += end - t
    return low


def main(path):
    opener = gzip.open if path.endswith(".gz") else open
    with opener(path, "rt") as f:
        edges = json.load(f)["logic_edges"]
    chans = {(c["peripheral"], c["pin"]): c for c in edges["channels"]}
    ch1n, ch4 = chans[("gpiod", 0)], chans[("gpioc", 4)]
    # Trimmed captures keep the newest edges; start after the latest first
    # edge so both channels are fully known, at a CH1N falling edge (CNT = 0,
    # OC1REF going active).
    first = max(c["transitions"][0]["cycle"] for c in (ch1n, ch4))
    end = min(c["transitions"][-1]["cycle"] for c in (ch1n, ch4))
    t0 = next(e["cycle"] for e in ch1n["transitions"] if e["cycle"] > first and e["value"] == 0)

    def initial_at(chan):
        before = [e for e in chan["transitions"] if e["cycle"] <= t0]
        return before[-1]["value"] if before else chan["initial"]

    init = {id(c): initial_at(c) for c in (ch1n, ch4)}
    runs = []  # [duty, periods]
    periods = 0
    lagging = 0
    start = t0
    while start + PERIOD <= end:
        stop = start + PERIOD
        d1, d4 = (low_time(c["transitions"], init[id(c)], start, stop) for c in (ch1n, ch4))
        want = (d1 + 128) & 255
        stepping = bool(runs) and runs[-1][0] != d1
        ok = {want, (want - 1) & 255} if stepping else {want}
        if d4 not in ok:
            sys.exit(f"FAIL period at {start}: CH1N low {d1}, CH4 low {d4}, want {want}")
        if stepping and d4 != want:
            lagging += 1
        if runs and runs[-1][0] != d1:
            if d1 != (runs[-1][0] + 1) & 255:
                sys.exit(f"FAIL at {start}: duty stepped {runs[-1][0]} -> {d1}, want +1")
            runs.append([d1, 0])
        elif not runs:
            runs.append([d1, 0])
        runs[-1][1] += 1
        periods += 1
        start = stop

    off_grid = [e["cycle"] for e in ch1n["transitions"]
                if t0 <= e["cycle"] < start and e["value"] == 0 and (e["cycle"] - t0) % PERIOD]
    if off_grid:
        sys.exit(f"FAIL: CH1N period start off the 256-cycle grid at {off_grid[:3]}")
    for duty, n in runs[1:-1]:
        if not HOLD - 1 <= n <= HOLD + 3:
            sys.exit(f"FAIL: duty {duty} held {n} periods, want ~{HOLD}")
    if len(runs) < 3:
        sys.exit(f"FAIL: only {len(runs)} duty values in the window")

    print(f"window: cycles {t0}..{start} ({periods} whole periods of {PERIOD} cycles)")
    for duty, n in runs:
        print(f"  CH1N low {duty:3}/256  CH4 low {(duty + 128) & 255:3}/256  for {n} periods")
    print(f"PASS: {periods} periods, {len(runs)} duty values, CH1N low = OC1REF high = count, "
          f"CH4 low = (count + 128) & 255, steps of +1 every ~{HOLD} periods (Delay_Ms(5)); "
          f"{lagging} step(s) where CH4 took the new count one period after CH1N")


if __name__ == "__main__":
    main(sys.argv[1])
