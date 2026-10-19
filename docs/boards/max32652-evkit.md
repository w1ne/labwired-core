# Validation Report: max32652-evkit

**Architecture:** ARM Cortex-M4F.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
[2m2026-10-18T14:40:04.263569Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting LabWired Simulator
[2m2026-10-18T14:40:04.264133Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../chips/onboarding/max32652-evkit.yaml
[2m2026-10-18T14:40:04.271494Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'max32650_gpio' (peripheral 'gpio0') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:40:04.271537Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'max32650_gpio' (peripheral 'gpio1') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:40:04.271552Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'max32650_gpio' (peripheral 'gpio2') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:40:04.271562Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'max32650_gpio' (peripheral 'gpio3') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:40:04.271716Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Loading firmware: "/home/andrii/Projects/labwired-renode/core/tests/fixtures/uart-ok-thumbv7m.elf"
[2m2026-10-18T14:40:04.271809Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x401
[2m2026-10-18T14:40:04.271820Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Firmware Loaded Successfully!
[2m2026-10-18T14:40:04.271825Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Entry Point: 0x401
[2m2026-10-18T14:40:04.273227Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting Simulation (ARM Cortex-M)...
[2m2026-10-18T14:40:04.273243Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Initial PC: 0x400, SP: 0x20020000
[2m2026-10-18T14:40:04.273248Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Running for 20000 steps...
[2m2026-10-18T14:40:04.303151Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Progress: 10000 steps, current IPS: 319304.59
[2m2026-10-18T14:40:04.316863Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Simulation loop finished.
[2m2026-10-18T14:40:04.316886Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Final PC: 0x466
[2m2026-10-18T14:40:04.316892Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Instructions: 19999
[2m2026-10-18T14:40:04.316898Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Cycles: 20002
[2m2026-10-18T14:40:04.316903Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Average IPS: 443700.35
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `led0` | `0x00000019` | `led` |
| `led1` | `0x0000001a` | `led` |
| `gcr` | `0x40000000` | `max32650_gcr` |
| `wdt0` | `0x40003000` | `max32650_wdt` |
| `wdt1` | `0x40003400` | `max32650_wdt` |
| `timer0` | `0x40010000` | `max32650_timer` |
| `timer1` | `0x40011000` | `max32650_timer` |
| `tpu` | `0x40001000` | `max32650_tpu` |
| `timer2` | `0x40012000` | `max32650_timer` |
| `timer3` | `0x40013000` | `max32650_timer` |
| `timer4` | `0x40014000` | `max32650_timer` |
| `timer5` | `0x40015000` | `max32650_timer` |
| `i2c0` | `0x4001d000` | `stub` |
| `i2c1` | `0x4001e000` | `stub` |
| `adc` | `0x40034000` | `stub` |
| `uart0` | `0x40042000` | `max32650_uart` |
| `uart1` | `0x40043000` | `max32650_uart` |
| `uart2` | `0x40044000` | `max32650_uart` |
| `spi0` | `0x40046000` | `stub` |
| `spi1` | `0x40047000` | `stub` |
| `spi2` | `0x40048000` | `stub` |
| `gpio0` | `0x40008000` | `max32650_gpio` |
| `gpio1` | `0x40009000` | `max32650_gpio` |
| `gpio2` | `0x4000a000` | `max32650_gpio` |
| `gpio3` | `0x4000b000` | `max32650_gpio` |
| `spi3` | `0x400be000` | `stub` |
| `rtc` | `0x40006000` | `stub` |
| `pwrseq` | `0x40006800` | `stub` |
| `dwt` | `0xe0001000` | `dwt` |
| `bitbandPeripherals` | `0x42000000` | `bitbanding` |
| `bitbandSram` | `0x22000000` | `bitbanding` |

**Total Peripherals:** 31