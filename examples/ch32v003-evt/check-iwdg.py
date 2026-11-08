#!/usr/bin/env python3
"""Check the watchdog reset of ch32fun's examples/iwdg.

Reads the `logic_edges` block of a `labwired test` result.json captured with
`--watch-gpio gpiod:0` and checks PD0's timeline against what the unmodified
example does at 48 MHz (Delay_Ms is SysTick-paced, 48,000 cycles per ms):

  boot:  on 1500 ms, off 1000 ms
         iwdg_setup(0xFFF, IWDG_Prescaler_128): started at t_start
         on 3000 ms, off, iwdg_feed() (reload from RLDR = 0xFFF)
         then on 250 ms / off 250 ms with no further feed
  reset: 4095 counts of LSI/128 = 4095 ms after the feed, the IWDG
         resets the chip. The CH32V003 GPIO returns to floating input, so
         PD0 drops if it was high, and the second boot starts:
         on 1500 ms, off ...

So the check is:

- the first boot's 1500/1000/3000 ms phases;
- 250 ms blink half-periods after the feed;
- the blink train ends at the feed + 4095 ms (one LSI count is 1 ms; the
  phase before the first count and the code between the feed and the
  deadline allow +/- 1 ms), and is followed by a 1500 ms on pulse:
  the second boot. A watchdog that never fired keeps blinking at 250 ms
  and fails here; one that fired at the wrong rate ends the train at the
  wrong time.

Edge times are compared with a tolerance of 1 ms (the instructions between
the Delay_Ms calls). The input may be gzipped. Exits non-zero on mismatch.
"""
import gzip
import json
import sys

MS = 48_000
TOL = 1 * MS
TIMEOUT_MS = 4095


def fail(msg):
    print(f"FAIL: {msg}")
    sys.exit(1)


def main(path):
    opener = gzip.open if path.endswith(".gz") else open
    with opener(path, "rt") as f:
        edges = json.load(f)["logic_edges"]
    chans = {(c["peripheral"], c["pin"]): c for c in edges["channels"]}
    pd0 = chans.get(("gpiod", 0))
    if pd0 is None:
        fail("no gpiod:0 channel in the capture")
    # Collapse same-cycle glitches and keep only real level changes.
    level = pd0["initial"]
    tl = []
    for e in pd0["transitions"]:
        if e["value"] != level:
            level = e["value"]
            if tl and tl[-1][0] == e["cycle"]:
                tl.pop()
            else:
                tl.append((e["cycle"], level))
    print(f"PD0: initial {pd0['initial']}, {len(tl)} level changes")
    for c, v in tl:
        print(f"  {c:>11} cycles  {c / MS:9.3f} ms  -> {v}")

    def expect(i, value, at, what):
        if i >= len(tl):
            fail(f"{what}: no edge #{i}")
        c, v = tl[i]
        if v != value or abs(c - at) > TOL:
            fail(f"{what}: edge #{i} is {v} at {c / MS:.3f} ms, "
                 f"expected {value} at {at / MS:.3f} ms")
        return c

    on1 = expect(0, 1, tl[0][0] if tl else 0, "first boot on")
    off1 = expect(1, 0, on1 + 1500 * MS, "first boot off after 1500 ms")
    on2 = expect(2, 1, off1 + 1000 * MS, "on after 1000 ms (IWDG started)")
    feed = expect(3, 0, on2 + 3000 * MS, "off after 3000 ms (iwdg_feed)")
    # gpios_off(); iwdg_feed(); then the loop's gpios_on() a few cycles
    # later: that on/off pair falls inside TOL, so the blink train starts
    # with an on edge just after `feed`.
    i = 4
    t = feed
    halves = 0
    deadline = feed + TIMEOUT_MS * MS
    value = 1
    while True:
        if i >= len(tl):
            fail("capture ended inside the blink train: no watchdog reset")
        c, v = tl[i]
        nominal = t + (250 * MS if halves else 0)
        if abs(c - nominal) <= TOL and v == value:
            t = c
            halves += 1
            value ^= 1
            i += 1
            continue
        break
    # The train ended. Either the reset dropped a high PD0 (the edge at the
    # deadline is a 0), or PD0 was low and the second boot's on is the next
    # edge (also at the deadline, give or take the boot code).
    print(f"blink train: {halves} edges after the feed, last at {t / MS:.3f} ms")
    c, v = tl[i]
    if abs(c - deadline) > 2 * TOL:
        fail(f"the blink train broke at {c / MS:.3f} ms, expected the watchdog "
             f"reset at feed + {TIMEOUT_MS} ms = {deadline / MS:.3f} ms")
    if v == 0:
        print(f"reset at {c / MS:.3f} ms released PD0 (GPIO back to input)")
        i += 1
        c, v = tl[i] if i < len(tl) else fail("no second boot after the reset")
    boot2 = c
    if v != 1 or abs(boot2 - deadline) > 2 * TOL:
        fail(f"second boot on is {v} at {boot2 / MS:.3f} ms")
    expect(i + 1, 0, boot2 + 1500 * MS, "second boot off after 1500 ms")
    print(f"feed at {feed / MS:.3f} ms, watchdog deadline "
          f"{deadline / MS:.3f} ms, second boot on at {boot2 / MS:.3f} ms "
          f"(+{(boot2 - feed) / MS:.3f} ms after the feed)")
    print("PASS")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(f"usage: {sys.argv[0]} result.json[.gz]")
    main(sys.argv[1])
