# ESP32-S2 Arduino matrix row — evidence notes (2026-11-24)

Board: `esp32s2` (`configs/chips/esp32s2-arduino.yaml`, PlatformIO
`esp32-s2-saola-1`, espressif32 8.0.0 = Arduino-ESP32 3.3.x / IDF 5.5).
Result: **9/9 cells pass** (`scoreboard-2026-11-24.md`).

This row was **red on `main` before this change**: L0, L1, L2, L4, L5 and L8
passed, L3, L6 and L7 did not. The board file nevertheless claimed all of
L2–L8, which is the failure mode this note records and fixes.

## What was measured before the fix

`validation/arduino-matrix/run_matrix.py --boards esp32s2 --sim-only` against
the CI-built ELFs of run 45395521286 (`arduino-matrix-esp32s2` artifact),
`labwired test` from branch content at `817ec19e18`:

| cell | symptom | UART |
|---|---|---|
| L3_i2c_sensor | `Memory access violation at 0xffffffff` at step 1,320,000, PC in `ina219_read_u16` | `LW_L3_BOOT` |
| L6_pwm | no marker inside 50,000,000 steps | `LW_L6_BOOT` |
| L7_timer | sketch's own verdict | `LW_L7_FAIL t0=2514 t1=5000 t2=4565 t3=5506` |

## Root cause: the caller's a4..a7 were lost across a window wrap

This is a CPU-core defect, not a missing peripheral. The peripherals involved
(SYSTIMER, I2C0, LEDC) all answered correctly:

* The systimer model's counter is strictly increasing. Tracing every S2
  SYSTIMER `UPDATE`/`VALUE_LO`/`VALUE_HI` access (`0x3F423000`) over the L7 run
  gives the samples 2505, 2514, 2516, 4505, 4506, 4565, 4567, 5505, 5506 µs —
  monotonic, and 4507 is what `micros()` should have returned for t1.
* `labwired test --breakpoint 0x4008154c` (the instruction after the second
  `micros()` call in `setup()`) shows `a3=2514 a7=4507 a10=4565`: the core held
  the correct value.
* The same breakpoint at `0x40081557` (after `delay(1)`) shows `a7=5000`. The
  value changed inside `call8 delay`; nothing else wrote it.
* The L3 fault address `0xffffffff` is a corrupted pointer in the same register
  group, and L6's missing marker is the same class of corruption reached
  through the LEDC setup path.

The Xtensa windowed ABI lets a caller keep a value in `a0..a7` across a call:
a `call8` rotates the window by 8, so the callee's `a0..a7` are the caller's
`a8..a15` and the caller's own `a0..a7` are physically untouched. When the call
chain wraps the 64-register file (eight nested `call8`s), the hardware raises
WindowOverflow and runs the firmware's `_WindowOverflow{4,8}` handler, which
spills the displaced frame to the stack.

`boot::esp32s2::fast_boot` left `faithful_windows` off, so the core ran the
sim's shadow-window mode instead. That mode saves **only a0..a3** of a displaced
window (`ArFile::push_shadow`), so a wrapped frame's `a4..a7` are not preserved.
The ESP32-S3 fast boot has run the firmware's own handlers since the Nano ESP32
work (`boot::esp32s3::prime_faithful_windows`); the S2 was the remaining core on
the shadow path.

## The change

`boot::esp32s2::fast_boot` now sets `cpu.faithful_windows = true` after it
primes the caller frame (`window 0`, `PS.CALLINC = 1`, SP) — the same frame
`prime_faithful_windows` builds, which is what `_WindowOverflow8` reads its
parent SP from. No descriptor or sketch changed.

Negative control, committed with the fix
(`crates/core/tests/esp/esp32s2_arduino_timer.rs`): booting the same L7 fixture
with `faithful_windows` forced back off reproduces `LW_L7_FAIL`, so the pass
depends on the window machinery and not on the sketch tolerating a bad clock.

## How this evidence was produced

```
python3 validation/arduino-matrix/run_matrix.py \
  --boards esp32s2 \
  --sketches L0_serial_boot,L1_serial_loop,L2_blink_serial,L3_i2c_sensor,\
L4_spi_sensor,L5_adc,L6_pwm,L7_timer,L8_can \
  --sim-only --out validation/arduino-matrix/out
```

`--sim-only` reuses the ELFs CI built for `esp32-s2-saola-1`. Committed here are
the oracle artifacts (`uart.log`, `junit.xml`) and the generated scoreboard.
`result.json` (≈170 kB per cell, dominated by the full register and peripheral
inspect dump) is not committed; re-run the command above to regenerate it.

## Still not claimed

No physical board ran any of this. Wi-Fi, USB OTG, PSRAM, I2C1, I2C slave, GP-SPI
DMA/slave, RMT RX, ADC calibration and touch remain unmodelled on this profile —
see the header of `configs/chips/esp32s2-arduino.yaml`.

## Vendor status

No new part is onboarded by this change: the ESP32-S2 profile and the
`esp32-s2-saola-1` row landed in `2c6495be17`; this repairs the row's claims.
Espressif's ESP32-S2 SoC page (`https://www.espressif.com/en/products/socs/esp32-s2`)
returned HTTP 200 on 2026-11-24 and carries no NRND or end-of-life notice. The
module itself (`ESP32-S2-WROOM`/-MINI) was not re-verified against a distributor
in this round.
