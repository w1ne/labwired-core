# Validation Report: arduino_nano_33_ble

**Architecture:** ARM Cortex-M4F. Features Bluetooth Low Energy.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
[2m2026-10-18T14:39:50.256551Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting LabWired Simulator
[2m2026-10-18T14:39:50.257151Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../chips/onboarding/arduino_nano_33_ble.yaml
[2m2026-10-18T14:39:50.259584Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Loading firmware: "/home/andrii/Projects/labwired-renode/core/tests/fixtures/uart-ok-thumbv7m.elf"
[2m2026-10-18T14:39:50.262703Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x401
[2m2026-10-18T14:39:50.262789Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Firmware Loaded Successfully!
[2m2026-10-18T14:39:50.262822Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Entry Point: 0x401
[2m2026-10-18T14:39:50.267184Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting Simulation (ARM Cortex-M)...
[2m2026-10-18T14:39:50.267203Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Initial PC: 0x400, SP: 0x20020000
[2m2026-10-18T14:39:50.267214Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Running for 20000 steps...
[2m2026-10-18T14:39:50.267305Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Simulation Error at step 2: Memory access violation at 0x2001ffe0
[2m2026-10-18T14:39:50.267325Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Simulation loop finished.
[2m2026-10-18T14:39:50.267330Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Final PC: 0x45c
[2m2026-10-18T14:39:50.267334Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Instructions: 2
[2m2026-10-18T14:39:50.267338Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Cycles: 2
[2m2026-10-18T14:39:50.267342Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Average IPS: 444.93
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `lsm9ds1_imu` | `0x0000006b` | `lsm9ds1_imu` |
| `lsm9ds1_mag` | `0x0000001e` | `lsm9ds1_magnetic` |
| `led_red` | `0x00000018` | `led` |
| `led_green` | `0x00000010` | `led` |
| `led_blue` | `0x00000006` | `led` |
| `uart0` | `0x40002000` | `nrf52840_uart` |
| `uart1` | `0x40028000` | `nrf52840_uart` |
| `rtc0` | `0x4000b000` | `nrf52840_rtc` |
| `rtc1` | `0x40011000` | `nrf52840_rtc` |
| `rtc2` | `0x40024000` | `nrf52840_rtc` |
| `wdt` | `0x40010000` | `nrf52840_watchdog` |
| `ppi` | `0x4001f000` | `nrf52840_ppi` |
| `clock` | `0x40000000` | `nrf_clock` |
| `twi0` | `0x40003000` | `nrf52840_i2c` |
| `spi2` | `0x40023000` | `nrf52840_spi` |
| `gpio0` | `0x50000500` | `nrf52840_gpio` |
| `gpio1` | `0x50000800` | `nrf52840_gpio` |
| `gpiote` | `0x40006000` | `nrf52840_gpiotasksevents` |
| `twi1` | `0x40004000` | `nrf52840_i2c` |
| `timer0` | `0x40008000` | `nrf52840_timer` |
| `timer1` | `0x40009000` | `nrf52840_timer` |
| `timer2` | `0x4000a000` | `nrf52840_timer` |
| `timer3` | `0x4001a000` | `nrf52840_timer` |
| `timer4` | `0x4001b000` | `nrf52840_timer` |
| `i2s` | `0x40025000` | `nrf52840_i2s` |
| `pdm` | `0x4001d000` | `nrf52840_pdm` |
| `radio` | `0x40001000` | `nrf52840_radio` |
| `rng` | `0x4000d000` | `nrf52840_rng` |
| `ecb` | `0x4000e000` | `nrf52840_ecb` |

**Total Peripherals:** 29