#!/usr/bin/env python3
"""Check that ch32fun's examples/tim2_pwm_remap moves TIM2's outputs.

Reads the `logic_edges` block of a `labwired test` result.json captured with
`--watch-gpio gpiod:4 --watch-gpio gpiod:3 --watch-gpio gpioc:1 --watch-gpio
gpioc:7` and checks what the unmodified example does (TIM2 at HCLK, PSC 0,
ATRLR 255, PWM mode 1, CC1P and CC2P set, so both channels are low while
CNT < CCRx):

- before ToggleRemap() (AFIO_PCFR1.TIM2_RM = 00) CH1 is on PD4 and CH2 on
  PD3, and PC1/PC7 carry nothing;
- after it (TIM2_RM = 11, full remap) CH1 is on PC1 and CH2 on PC7, and PD4
  and PD3 stop: the switch happens within one 256-cycle period, with no pad
  pair running at once for longer than that;
- on both pairs, every whole period is 256 cycles and CH1 low + CH2 low =
  255 (CH1CVR = count, CH2CVR = 255 - count), except for the one period
  after a step where CH2 may still carry the previous count (the two stores
  land on either side of an update event). t2pwm_setpw() sets UG after
  each store, restarting the counter, so a period cut short is allowed
  only where the count steps;
- `count` steps by +1 each Delay_Ms(5) (937.5 periods) and restarts at 0 on
  the remapped pads.

The capture keeps the newest ~18-33 k edges per pad, so the window starts at
the latest pad's first captured edge. The input may be gzipped. Prints a
summary and exits non-zero on the first mismatch.
"""
import gzip
import json
import sys

PERIOD = 256
HOLD = 48_000_000 * 5 // 1000 / PERIOD  # periods per Delay_Ms(5)


def edges_of(chan):
    """(cycle, value) pairs, value 0/1; tri-state captures map 'z' to None."""
    if "transitions" in chan:
        return [(e["cycle"], e["value"]) for e in chan["transitions"]]
    out = []
    for e in chan.get("states", []):
        out.append((e["cycle"], {"0": 0, "1": 1}.get(e["state"])))
    return out


def low_time(edges, start, end):
    level = None
    for c, v in edges:
        if c > start:
            break
        level = v
    t = start
    low = 0
    for c, v in edges:
        if c <= start:
            continue
        if c >= end:
            break
        if level == 0:
            low += c - t
        t, level = c, v
    if level == 0:
        low += end - t
    return low


def decode(name, ch1, ch2, start, end):
    """Duty runs of one pad pair, one period per pair of CH2 falling edges.

    CH2CVR = 255 - count is never 0, so CH2 falls at every counter restart:
    each overflow (CNT 255 -> 0), and each UG, which t2pwm_setpw() sets after
    every compare store. A UG lands mid-period, so the period it cuts short
    is not 256 cycles; those may only appear where the count steps."""
    falls = [c for c, v in ch2 if start <= c <= end and v == 0]
    runs = []
    short = 0
    lag = 0
    for a, b in zip(falls, falls[1:]):
        d1, d2 = low_time(ch1, a, b), low_time(ch2, a, b)
        if b - a != PERIOD:
            # A UG restart: allowed only between two runs (checked below).
            short += 1
            if runs:
                runs[-1].append("cut")
            continue
        stepping = bool(runs) and runs[-1][0] != d1
        if d1 + d2 != 255 and not (stepping and d1 + d2 == 256):
            sys.exit(f"FAIL {name} period at {a}: CH1 low {d1} + CH2 low {d2} != 255")
        if stepping:
            if d1 != runs[-1][0] + 1:
                sys.exit(f"FAIL {name} at {a}: count stepped {runs[-1][0]} -> {d1}, want +1")
            lag += d1 + d2 == 256
            runs.append([d1, 0])
        elif not runs:
            runs.append([d1, 0])
        elif "cut" in runs[-1]:
            sys.exit(f"FAIL {name} at {a}: a short period inside count {d1}'s run")
        runs[-1][1] += 1
    for run in runs[1:-1]:
        if not HOLD - 2 <= run[1] <= HOLD + 3:
            sys.exit(f"FAIL {name}: count {run[0]} held {run[1]} periods, want ~{HOLD}")
    runs = [r[:2] for r in runs]
    return falls[0], falls[-1], runs, lag, short


def main(path):
    opener = gzip.open if path.endswith(".gz") else open
    with opener(path, "rt") as f:
        cap = json.load(f)["logic_edges"]
    chans = {(c["peripheral"], c["pin"]): edges_of(c) for c in cap["channels"]}
    pd4, pd3 = chans[("gpiod", 4)], chans[("gpiod", 3)]
    pc1, pc7 = chans[("gpioc", 1)], chans[("gpioc", 7)]
    if not (pd4 and pd3 and pc7):
        sys.exit("FAIL: a pad recorded no edges")
    window = max(pd4[0][0], pd3[0][0])
    old_last = max(pd4[-1][0], pd3[-1][0])
    new_first = min(pc1[0][0], pc7[0][0])
    early = [c for c, _ in pc1 + pc7 if window <= c < new_first]
    if early:
        sys.exit(f"FAIL: PC1/PC7 moved at {early[:3]} while TIM2_RM = 00")
    if not -PERIOD <= new_first - old_last <= PERIOD:
        sys.exit(f"FAIL: PD4/PD3 last edge {old_last}, PC7 first edge {new_first}: "
                 "not a switch within one period")
    a0, a1, old_runs, old_lag, old_short = decode("PD4/PD3", pd4, pd3, window, old_last)
    b0, b1, new_runs, new_lag, new_short = decode("PC1/PC7", pc1, pc7, new_first, cap["now_cycle"])
    if old_runs[-1][0] != 254:
        sys.exit(f"FAIL: PD4/PD3 ended at count {old_runs[-1][0]}, want 254 (the loop's last)")
    if new_runs[0][0] != 0:
        sys.exit(f"FAIL: PC1/PC7 started at count {new_runs[0][0]}, want 0")
    print(f"TIM2_RM = 00, PD4 (CH1) / PD3 (CH2): cycles {a0}..{a1}, counts "
          f"{old_runs[0][0]}..{old_runs[-1][0]} ({sum(n for _, n in old_runs)} periods)")
    print(f"  last PD4/PD3 edge {old_last}; PC1/PC7 flat over {window}..{new_first}")
    print(f"TIM2_RM = 11, PC1 (CH1) / PC7 (CH2): cycles {b0}..{b1}, counts "
          f"{new_runs[0][0]}..{new_runs[-1][0]} ({sum(n for _, n in new_runs)} periods)")
    print(f"  first PC1/PC7 edge {new_first}, {new_first - old_last} cycles after the last PD edge")
    for name, runs in (("PD4/PD3", old_runs[-3:]), ("PC1/PC7", new_runs[:3])):
        for duty, n in runs:
            print(f"  {name}: CH1 low {duty:3}/256  CH2 low {255 - duty:3}/256  for {n} periods")
    print(f"PASS: the pair moves PD4/PD3 -> PC1/PC7 at the remap, every whole period 256 cycles "
          f"with CH1 low + CH2 low = 255, count +1 every ~{HOLD} periods; "
          f"{old_short + new_short} period(s) cut short by a UG restart, all at a count step; "
          f"{old_lag + new_lag} step(s) where CH2 took the new count one period late")


if __name__ == "__main__":
    main(sys.argv[1])
