# nRF5340 TIMER channel count and nRF52832 TIMER1/2 negative control (2026-11-24)

Two descriptor gaps in the same gate, reverted together with the fixed test
binary:

1. `nrf5340.yaml` here is the fixed file with the three `num_cc: 6` lines
   removed. The model then keeps its default of 4 compare channels.
2. `nrf52832.yaml` here is the pre-change descriptor: TIMER0 only, no
   TIMER1/TIMER2.

`cargo-test.log` shows both failures:

```
timer at 0x5000f000: CC[5] write was dropped (num_cc < 6?)
called `Result::unwrap()` on an `Err` value: MemoryViolation(1073779976)
```

`1073779976` is `0x4000_9008`: the PRESCALER register of TIMER1, which the old
nRF52832 descriptor did not map.

Evidence for the fix: the vendored SVD
`configs/peripherals/nrf5340/timer0_s.yaml` lists `TASKS_CAPTURE0..5` and
`EVENTS_COMPARE0..5`; the Nordic MDK says `TIMER0_CC_NUM 6`. nRF52832 PS
§6.30 places TIMER0/1/2 at `0x4000_8000/9000/A000` on IRQ 8/9/10.

With the fixed descriptors the gate passes 3/3: see
`../2026-11-24-negative-control-timer-base-clock/after.log`.
