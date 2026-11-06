#!/usr/bin/env python3
"""Check the TIM2 PWM waveform of ch32fun's examples/tim2_pwm.

Reads the `logic_edges` block of a `labwired test` result.json captured with
`--watch-gpio gpiod:4 --watch-gpio gpiod:3 --watch-gpio gpiod:7` and checks,
for every whole PWM period in the captured window, what the unmodified
example programs (TIM2 at HCLK, PSC 0, ATRLR 255, PWM mode 1):

- the period is 256 timer counts (256 CPU cycles at 48 MHz, 187.5 kHz);
- CH1 (PD4) is low for `count` cycles per period and CH2 (PD3) is low for
  `(count + 128) & 255` cycles, the example's 180-degree offset: both have
  CCxP set (`TIM_CCxP & TIM2_DEFAULT`), so they idle high and pull low;
- CH4 (PD7) is HIGH for `(count + 128) & 255` cycles: the example's
  `TIM_CC4P & TIM2_DEFAULT` is `0x2000 & 0xff` = 0, so CH4 keeps the reset
  active-high polarity, exactly as it would on silicon;
- at a step, CH2/CH4 may lag CH1 by one period (the stores are in that order
  and the compare registers are preloaded), never longer and never CH4
  ahead of CH2;
- `count` only ever steps by +1 (mod 256), and each value is held for
  Delay_Ms(5) worth of periods (240,000 cycles / 256 = 937.5).

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
    ch1, ch2, ch4 = (chans[("gpiod", p)] for p in (4, 3, 7))
    # Trimmed captures keep the newest edges; start after the latest first
    # edge so every channel is fully known, at a CH1 falling edge (CNT = 0).
    first = max(c["transitions"][0]["cycle"] for c in (ch1, ch2, ch4))
    end = min(c["transitions"][-1]["cycle"] for c in (ch1, ch2, ch4))
    t0 = next(e["cycle"] for e in ch1["transitions"] if e["cycle"] > first and e["value"] == 0)

    def initial_at(chan):
        before = [e for e in chan["transitions"] if e["cycle"] <= t0]
        return before[-1]["value"] if before else chan["initial"]

    init = {id(c): initial_at(c) for c in (ch1, ch2, ch4)}
    runs = []  # [duty, periods]
    periods = 0
    lagging = 0
    start = t0
    while start + PERIOD <= end:
        stop = start + PERIOD
        d1, d2, d4 = (low_time(c["transitions"], init[id(c)], start, stop) for c in (ch1, ch2, ch4))
        want = (d1 + 128) & 255
        # The loop stores CH1CVR, CH2CVR, CH4CVR in that order and OCxPE
        # preloads them, so an update event between the stores gives one
        # period where CH2/CH4 still carry the previous count.
        stepping = bool(runs) and runs[-1][0] != d1
        ok = {want, (want - 1) & 255} if stepping else {want}
        if d2 not in ok or PERIOD - d4 not in ok or (d2 != want and PERIOD - d4 == want):
            sys.exit(f"FAIL period at {start}: CH1 low {d1}, CH2 low {d2}, CH4 high {PERIOD - d4}, "
                     f"want {want}")
        if stepping and (d2 != want or PERIOD - d4 != want):
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

    # CH1 falling edges must sit exactly on the 256-cycle grid from t0.
    off_grid = [e["cycle"] for e in ch1["transitions"]
                if t0 <= e["cycle"] < start and e["value"] == 0 and (e["cycle"] - t0) % PERIOD]
    if off_grid:
        sys.exit(f"FAIL: CH1 period start off the 256-cycle grid at {off_grid[:3]}")
    # Interior runs (not the clipped first/last) last one Delay_Ms(5) plus
    # the loop's few instructions, i.e. 937-940 periods.
    for duty, n in runs[1:-1]:
        if not HOLD - 1 <= n <= HOLD + 3:
            sys.exit(f"FAIL: duty {duty} held {n} periods, want ~{HOLD}")

    print(f"window: cycles {t0}..{start} ({periods} whole periods of {PERIOD} cycles)")
    for duty, n in runs:
        w = (duty + 128) & 255
        print(f"  CH1 low {duty:3}/256  CH2 low {w:3}/256  CH4 high {w:3}/256  for {n} periods")
    print(f"PASS: {periods} periods, {len(runs)} duty values, CH2 low = CH4 high = (CH1 low + 128) & 255, "
          f"steps of +1 every ~{HOLD} periods (Delay_Ms(5)); "
          f"{lagging} step(s) where CH2/CH4 took the new count one period after CH1")


if __name__ == "__main__":
    main(sys.argv[1])
