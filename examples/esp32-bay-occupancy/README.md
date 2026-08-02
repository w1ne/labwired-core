# Bay occupancy — four VCNL4010s behind a TCA9548A

A real customer topology, verified end to end in the twin.

```
Adafruit ESP32 Feather V2 (PID 5400, ESP32-D0WD-V3)
  ├── TCA9548A I²C switch @ 0x70 (PID 2717)
  │     └── ch0..ch3 → VCNL4010 @ 0x13 (PID 466), one per channel
  └── 2.4" TFT FeatherWing V2 (PID 3315), ILI9341 over SPI
```

**Four VCNL4010s cannot share a bus.** The address is fixed at `0x13` in
silicon with no strap pins, so the switch is not a convenience — it is the
entire reason this topology exists. Every sensor access goes through
`selectChannel()` first, and that is the only place the switch is written, so
channel isolation is a property of construction rather than of luck.

## What is here

| path | what |
|---|---|
| `firmware/` | the application: thresholds, hysteresis, debounce, per-bay display, fault reporting, non-blocking scheduling |
| `system.yaml` | the full rig |
| `system-sensor-missing.yaml` | same rig with bay 3 absent — the fixture for the fault tests |
| `tests/` | one declarative script per behaviour, plus a runner |

## Running it

```sh
cd firmware && pio run && cd ../tests && ./run-all.sh
```

Expected:

```
PASS  1,2,3   init, per-read channel select, independent injection
PASS  4       raw counts -> EMPTY/PRESENT via thresholds
PASS  5       separate entry/exit thresholds (hysteresis)
PASS  6       debounce: single noisy spike rejected
PASS  7       simultaneous occupancy across all four bays
PASS  8       one channel cannot change another's state
PASS  9       missing sensor / I2C failure detected
PASS  10      per-bay state painted on the TFT              (panel ink=6610)
PASS  11      clear FAULT state for an unreadable sensor    (panel ink=6610)
PASS  12      polling and display do not block each other   (panel ink=6352)
```

## The three assertions worth understanding

These were chosen so that a broken implementation cannot pass them by accident.

**Hysteresis (test 5)** asserts the single line:

```
STATE P2000 E2000
```

Bay 0 reads PRESENT and bay 1 reads EMPTY at *the same count of 2000*,
differing only by which side each arrived from. A single-threshold
implementation cannot produce that line at any threshold value.

**Fault (test 11)** asserts:

```
STATE E2000 E2000 E2000 F0
```

The bay with no sensor publishes `F`, never a confident `E`. An unmonitored
bay that *looks free* is the dangerous failure mode, so the firmware refuses to
report one — a dead sensor reads as `0` or `0xFFFF` on the wire, and both rails
are treated as "not a measurement" rather than as an empty bay.

**Non-blocking (test 12)** asserts polling is alive both *before and after* a
mid-run sensor change, and that the panel painted, in the same execution. The
failure modes are asymmetric: if the display blocked polling the STATE lines
stall; if polling blocked the display the panel never finishes. Asserting both
catches either.

## Two limitations, stated plainly

**Display assertions run outside the script.** `labwired test` has no `display`
clause — its `TestAssertion` is `uart_contains` / `uart_matches` /
`expected_stop_reason` / `memory_value` / `uds_tester` — and `result.json`'s
`inspect` block returns zero artifacts even on a run that demonstrably painted.
So `run-all.sh` takes the painted-byte count from `snapshot capture`, which does
expose the panel. That is honest evidence, but it means a user's own test script
cannot yet check their display.

**This is a logic oracle, not a timing oracle.** Benchmarked against a real
ESP32-D0WDQ6, the twin's per-operation costs are 2×–89× off silicon. State
machines, thresholds and channel isolation are trustworthy here; wall-clock
durations are not.
