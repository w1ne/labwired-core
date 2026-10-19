# Periodic CAN timing, measured cycle-exact

The `f103-can-dashboard` engine firmware (`../f103-can-dashboard/prebuilt/engine.elf`)
transmits its RPM frame (CAN id `0x100`) from a TIM2 periodic interrupt: a
100 µs timer tick with a 10 ms period. This example captures those frames on
virtual time through a `can-bridge` and measures the schedule's period, the
question behind Renode #674 ("sleep/timing: the sim's clock does not move at
the rate the firmware asked for").

```bash
# from the repository root
cargo run -p labwired-cli -- test \
  --script examples/periodic-can-timing/script.yaml \
  --output-dir /tmp/labwired-periodic
python3 examples/periodic-can-timing/measure.py /tmp/labwired-periodic/can-bus.jsonl 100
```

## What you should see

```
frames: 108  (id 0x100, tx)
inter-frame deltas: {720003: 107}
period: 720003 cycles = 10.000042 ms (constant to the cycle)
100 intervals: 72000300 cycles = 1.000004167 s virtual
```

- At the F103's 72 MHz core clock, a 10 ms period is 720,000 cycles. The
  recorded interval is **720,003** cycles — the 3 cycles are the firmware's
  deterministic cost between the timer tick and the CAN transmit-request
  write (restart the timer, load the mailbox). Every one of the 107 intervals
  carries the same count: **zero jitter**, no drift across 100 intervals.
- The virtual span of 100 intervals is exactly **1.000004167 s** — the same
  number every run, on any host. The run itself finishes in milliseconds:
  `idle_ff` proves the timer wait is idle and fast-forwards it (you will see
  `idle_ff skipped … of … device cycles` on stderr), while the virtual
  timeline stays exact — the frames arrive at the cycles silicon would put
  them on.
- The first frame leaves at cycle 720,925 and the second at 1,440,928 — at
  72 MHz those instants are 10,012.85 µs and 20,012.89 µs. (`can-bus.jsonl`
  carries the cycles; `result.json`'s `can_bridges[0].timeline[]` carries the
  same instants as `t_us` for readers who prefer microseconds.)

## Re-running against another firmware

Change `inputs.firmware:` in `script.yaml` and the CAN id in `measure.py`
(argument-free by design — edit the constant if your frame id differs).
