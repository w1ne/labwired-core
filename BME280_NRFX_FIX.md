# BME280 Zephyr ztest Fix: nRF52 TWIM + RTC Simulator Bugs

## Summary

Two bugs prevented a real Zephyr BME280 sensor-read ztest from passing inside the
LabWired nRF52840 simulator.  Both have been fixed.  Final result:

```
Running TESTSUITE bme280_read
 PASS - test_device_ready in 0.000 seconds
 PASS - test_fetch_temperature_in_range in 0.005 seconds
TESTSUITE bme280_read succeeded
------ TESTSUITE SUMMARY START ------
SUITE PASS - 100.00% [bme280_read]: pass = 2, fail = 0, skip = 0, total = 2 duration = 0.005 seconds
PROJECT EXECUTION SUCCESSFUL
```

---

## Bug 1: Spurious Double-ISR after NVIC ICPR Write

### File
`crates/core/src/cpu/cortex_m.rs` and `crates/core/src/bus/accessors.rs`

### Root cause

When a peripheral `tick()` fired during ISR execution it re-asserted the interrupt
line, which set the corresponding bit in `cpu.pending_exceptions`.  The nrfx driver
then wrote to NVIC ICPR (Interrupt Clear-Pending Register) — normally used to clear
a pending exception — but the simulator only cleared the NVIC ISPR shadow register.
The stale bit in `cpu.pending_exceptions` was never cleared.

After the ISR returned, the CPU saw the stale pending bit and immediately re-entered
the same ISR.  The spurious second invocation corrupted the nrfx state machine
(e.g. calling the ISR with no real event pending caused it to exit without signalling
the `k_sem` that the transfer was complete).

### Fix

Added `is_nvic_irq_pending()` to the `Bus` trait (default: `true` for backward
compatibility).  The `SystemBus` implementation checks NVIC ISPR for the relevant
bit.  In `step_internal`, before taking an exception from `pending_exceptions`, the
CPU calls `is_nvic_irq_pending()`.  If the ISPR bit is clear (ICPR write already
cleared it) the stale `pending_exceptions` bit is dropped silently without taking
the exception.

---

## Bug 2: RTC Running 1953× Too Fast (LFCLK Rate Not Modelled)

### File
`crates/core/src/peripherals/nrf52/rtc.rs`

### Root cause

The nRF52840 RTC peripheral runs on the LFCLK (Low-Frequency Clock) at 32,768 Hz,
not on the 64 MHz CPU clock.  The simulator's `tick()` method is called once per
CPU cycle.  Before this fix, the RTC `tick()` advanced the RTC counter every CPU
cycle — 1,953× faster than the real hardware.

Zephyr uses RTC1 (0x40011000, IRQ 17) as its system clock via `sys_clock_driver_init`.
With PRESCALER=0 the RTC fires EVENTS_COMPARE every 32768 CPU cycles for a 1 ms
Zephyr tick.  At 1953× speed the simulated millisecond elapsed in only 16 CPU
cycles.

The nrfx TWIM driver calls `k_sem_take(&dev_data->completion_sync, I2C_TRANSFER_TIMEOUT_MSEC)`
with `I2C_TRANSFER_TIMEOUT_MSEC = K_MSEC(500)`.  The 24-byte BME280 calibration
read requires ~144,000 CPU cycles to complete.  With the inflated clock, Zephyr's
scheduler believed 500 ms had passed after only ~16,500 CPU cycles — the semaphore
timed out before the transfer finished.  `bme280_chip_init` returned an error,
leaving the device not ready.

### Fix

Added a fractional LFCLK accumulator to `Nrf52Rtc`:

```
64_000_000 / 32_768 = 1953.125 = 15625 / 8  (exact, no rounding)
```

The accumulator increments by 8 every CPU cycle (every `tick()` call).  When it
reaches 15,625 a single LFCLK base-clock edge fires; the existing PRESCALER divider
logic then runs as before.  This gives exactly 32,768 Hz without accumulating any
rounding error.

New constants:
```rust
pub const LFCLK_ACCUM_INC_DEFAULT: u32 = 8;
pub const LFCLK_ACCUM_PERIOD_DEFAULT: u32 = 15625;
```

New fields on `Nrf52Rtc`:
```rust
lfclk_accum: u32,  // running total
lfclk_inc: u32,    // default 8
lfclk_period: u32, // default 15625
```

A `new_fast()` constructor (cfg(test) only) sets both to 1, giving 1:1 ratio so
unit tests that call `tick()` directly keep using small tick counts.  The integration
test `nrf52840_onboarding_rtc0_fires_compare_and_pends_irq` was updated to tick
10,000 times (CC[0]=4 requires ~7,812 CPU ticks at real rate).

---

## Test coverage preserved

All 37 targeted tests remain green after both fixes:

- 29 TWIM unit tests
- 7 serial_instance tests
- 1 bme280 unit test
- Full `cargo test -p labwired-core --release` passes (0 failures)

---

## Firmware / system under test

- ELF: `zephyr.elf` — Zephyr v3.x with `CONFIG_I2C_NRFX=y`, BME280 driver,
  2 ztests (`test_device_ready`, `test_fetch_temperature_in_range`)
- System: nRF52840 with TWIM0 wired to a BME280 sensor model in the lab YAML
- Run: `labwired --firmware zephyr.elf --system sys.yaml --max-steps 8000000`
