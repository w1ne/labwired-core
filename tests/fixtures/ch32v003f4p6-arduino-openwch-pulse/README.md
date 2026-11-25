# CH32V003F4P6 openwch Arduino pulse-width fixture

Built with the same settings as `../ch32v003f4p6-arduino-openwch/`
(PlatformIO `ch32v` platform 1.1.0, `board = ch32v003f4p6_evt_r0`,
`framework = arduino`, `board_build.core = openwch`,
`framework-arduino-openwch-ch32`); this fixture has its own sketch. The sketch
is not modified for the simulator.

What the sketch exercises:

- an external pulse on **PD3**, which the sketch only ever reads
  (`pinMode(PD3, INPUT)`; the harness drives the pad). No output edge comes
  from the firmware, so every measured width is the harness's.
- `SysTick->CNT` on the QingKe V2 STK, which counts up from 0 to `CMP` and
  reloads (`CMP = 48 000 - 1`, `CTLR = 0x0F` from the core's `hw_config.c`).
  `stk_micros()` reads tick -> counter -> tick and retries if the millisecond
  rolled over, so a sample cannot mix two milliseconds. Printed values are
  `WIDTH <us>` for a 100 us and a 250 us pulse.
- the stock core's `micros()`: three consecutive samples are printed on one
  `MICROS` line. On this variant `getCurrentMicros()` (`ch32/clock.c`)
  computes `(CMP + 1 - CNT) * 1000 / (CMP + 1)`, which is the *remaining*
  time in the millisecond for an up-counter, so the samples decrease.
- `pulseIn(PD3, HIGH, 20000)`, timed with the STK. It measures with
  `micros()`, so the timeout test rejects the call at once: the sketch
  prints `PULSEIN 0 <call us>`, and the call ends before the 100 us pulse
  the harness drives after `PULSEIN_WAIT` can finish.

The tests in `crates/core/tests/vendor/ch32v003_compiled_firmware.rs` place
each pulse on exact HCLK cycle boundaries (4 800 cycles = 100 us, 12 000 =
250 us at 48 MHz) and compare the printed microseconds with the driven width.

Rebuild:

```
pio run -d tests/fixtures/ch32v003f4p6-arduino-openwch-pulse
cp tests/fixtures/ch32v003f4p6-arduino-openwch-pulse/.pio/build/sim/firmware.elf \
   tests/fixtures/ch32v003f4p6-arduino-openwch-pulse.elf
```
