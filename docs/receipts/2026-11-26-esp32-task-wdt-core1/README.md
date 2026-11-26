# ESP32 (classic): stock Arduino task watchdog fires and core 1 survives it

**Demand rank:** the ESP32 family is **#1** in the JetBrains State of Embedded
2025 MCU survey (38 % of respondents; 43 % among enthusiasts). This is depth
work on a part the engine already boots, not a new part. It follows the TIMG
main-system watchdog model (`docs/receipts/2026-11-26-esp32-classic-mwdt`) and
exercises it with stock Arduino-ESP32 firmware instead of a register-level fixture.

**Board:** a bare ESP32-WROOM-32 (`configs/systems/esp32-wroom-32.yaml`),
`board = esp32dev`. Vendor status checked live on 2026-11-26: Espressif's
[module catalog](https://www.espressif.com/en/products/modules/esp32/wroom-32)
lists ESP32-WROOM-32E-N4/N8/N16 with DigiKey and Mouser purchase links, and its
[longevity commitment](https://www.espressif.com/en/products/longevity-commitment)
lists the ESP32 for at least 15 years starting 2016-01-01.

## Result

Stock Arduino-ESP32 3.x (`framework-arduinoespressif32-esp32-libs`, ESP-IDF
v5.5.5) sketch, `crates/cli/tests/fixtures/esp32_arduino_task_wdt`:

| phase | before (`origin/main` `98d870826d`) | after |
|---|---|---|
| feed TWDT for 300 ms (3 timeouts) | `LW_TWDT_FED_OK` | `LW_TWDT_FED_OK` |
| stop feeding | `E (801) task_wdt: Task watchdog got triggered` ... `- loopTask (CPU 1)` | same |
| core 1 services the backtrace request | nothing printed, core 1 halted | `Backtrace: 0x4008217E:0x3FFBFB7C` |
| `loopTask` sees the ISR flag | never: run exhausts **200,000,000** steps | `LW_TWDT_FIRED`, PASS 4/4 checks, **96,490,000 steps, 9.7 s** |

The TIMG0 watchdog model was already correct: the TWDT fired at the right
time on both binaries. The defect was in the CPU model on core 1.

## Root cause

The TWDT ISR on core 0 calls `esp_crosscore_int_send_print_backtrace(1)`.
Core 1 takes the cross-core interrupt and runs `esp_backtrace_print`, which
calls `xthal_window_spill` and then reads the caller frames from the stack with
`l32e`.

The classic `labwired test` path ran both cores in shadow-window mode. That
mode keeps displaced register windows in a host-side shadow stack, not in the
guest stack save areas, and it replaced `xthal_window_spill_nw` with a thunk
that spills from that shadow stack. The backtrace walk then read frames that
were never on the stack. Diagnosis, in order:

1. `before/stuck_loop_uart.log`: the TWDT report repeats every 100 ms, core 1
   prints no backtrace, and the report changes from `CPU 1: loopTask` to
   `CPU 1: IDLE1`. `LW_TWDT_FIRED` never appears.
2. `diag/monitor.cpp` -> `before/monitor_uart.log`: a monitor task on core 0
   prints a counter that the spin loop on core 1 increments. The counter stops
   at the first TWDT report (`spin=199765`) and stays there.
3. `diag/crosscore_bt.cpp` -> `before/crosscore_bt_uart.log`: no watchdog;
   core 0 sends a cross-core *yield* (core 1 keeps running) and then a
   cross-core *print backtrace*. The run stops with `stop_reason: exception`
   at 38,990,000 steps. After the change, `after/crosscore_bt_uart.log`
   prints the backtrace and the counter keeps rising (`spin=11964056`).

## What changed

`crates/cli/src/commands/test.rs`, classic ESP32 manifest branch: both cores
now run faithful window overflow/underflow (`prime_faithful_windows`), as the
ESP32-S3 branch has done since the Nano ESP32 row. The shadow-only
`xthal_window_spill_nw` thunk is no longer installed on this path, because the
firmware's own spill routine is correct under faithful windows. The helper that
installed it (`install_xtensa_freertos_workarounds`) had no other caller and is
removed. The opt-in `inputs.profile: arduino-esp32` fast boot is unchanged: it
installs its own spill thunk and keeps shadow mode.

## Regression evidence (after)

- Arduino matrix ESP32 row, recompiled against the current core:
  `run_matrix.py --boards esp32` -> **9 pass / 0 skip / 0 fail** (L0-L8).
- `labwired test` scripts on classic ESP32 systems, all PASS:
  `esp32-classic-wdt/wdt_starve` 2/2, `wdt_feed` 1/1;
  `esp32-devkitc-32e-uart1` io-smoke 2/2, tier1-smoke 2/2, uart1-smoke 3/3;
  `esp32-devkitc-32e-gpio-smoke` io-smoke 2/2, tier1-smoke 2/2;
  `esp32-devkitm-1` onboarding-smoke 3/3, bme280-smoke 5/5;
  `adafruit-feather-esp32-v2` io-smoke 1/1, bme280-smoke 7/7;
  `esp32-bay-occupancy` (Arduino app, 3,000,000,000-4,000,000,000 steps
  each): test-debounce 2/2, test-fault-and-display 5/5, test-nonblocking 3/3,
  test-occupancy-combinations 3/3, test-thresholds-hysteresis 5/5.
- New PR-gated test: `cargo test -p labwired-cli --test esp32_arduino_task_wdt`.

## Not covered (not claimed)

- **The backtrace is one frame deep.** Core 1 prints
  `Backtrace: 0x4008217E:0x3FFBFB7C |<-CORRUPTED`. On silicon the walk
  continues through the interrupt dispatcher into `loopTask`. The frame after
  `esp_crosscore_isr` does not pass `esp_backtrace_get_next_frame`'s sanity
  check, so a window or interrupt-frame detail is still not faithful. The test
  asserts only that core 1 prints a backtrace and returns, not its depth.
- `trigger_panic = true` (the Arduino default) is not exercised: the panic
  path ends in a system reset, and MWDT stage 2/3 resets are not modelled.
- `labwired run` and the playground (`new_from_config_xtensa_esp32`) classic
  paths still run shadow-window mode; only `labwired test` changed.
- Two cores calling `esp_rom_printf` at once interleave bytes on UART0. That
  is firmware behaviour (no lock), not a simulator defect.
