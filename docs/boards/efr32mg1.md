# Validation Report: efr32mg1

**Architecture:** ARM Cortex-M4F.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
[2m2026-10-18T14:39:55.060447Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting LabWired Simulator
[2m2026-10-18T14:39:55.063385Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../chips/onboarding/efr32mg1.yaml
[2m2026-10-18T14:39:55.064499Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'efr32_gpioport' (peripheral 'gpioPort') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:39:55.064727Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Loading firmware: "/home/andrii/Projects/labwired-renode/core/tests/fixtures/uart-ok-thumbv7m.elf"
[2m2026-10-18T14:39:55.064856Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x401
[2m2026-10-18T14:39:55.064899Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Firmware Loaded Successfully!
[2m2026-10-18T14:39:55.064906Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Entry Point: 0x401
[2m2026-10-18T14:39:55.065734Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting Simulation (ARM Cortex-M)...
[2m2026-10-18T14:39:55.065742Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Initial PC: 0x400, SP: 0x20020000
[2m2026-10-18T14:39:55.065747Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Running for 20000 steps...
[2m2026-10-18T14:39:55.065825Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Simulation Error at step 2: Memory access violation at 0x2001ffe0
[2m2026-10-18T14:39:55.065844Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Simulation loop finished.
[2m2026-10-18T14:39:55.065849Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Final PC: 0x45c
[2m2026-10-18T14:39:55.065853Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Instructions: 2
[2m2026-10-18T14:39:55.065857Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Cycles: 2
[2m2026-10-18T14:39:55.065860Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Average IPS: 2104.16
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `i2c0` | `0x4000c000` | `stub` |
| `usart0` | `0x40010000` | `efr32_usart` |
| `usart1` | `0x40010400` | `efr32_usart` |
| `leUart0` | `0x4004a000` | `leuart` |
| `gpioPort` | `0x4000a000` | `efr32_gpioport` |
| `bitband_peripherals` | `0x42000000` | `bitbanding` |
| `bitclear` | `0x44000000` | `stub` |
| `bitset` | `0x46000000` | `stub` |
| `bitband_sram` | `0x22000000` | `bitbanding` |
| `timer0` | `0x40018000` | `efr32_timer` |
| `timer1` | `0x40018400` | `efr32_timer` |
| `cmu` | `0x400e4000` | `stub` |
| `ldma` | `0x400e2000` | `efr32mg12_ldma` |

**Total Peripherals:** 13