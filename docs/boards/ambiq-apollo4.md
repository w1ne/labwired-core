# Validation Report: ambiq-apollo4

**Architecture:** ARM Cortex-M4F. Ultra-low power MCU with SPOT technology.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
[2m2026-10-18T14:39:50.033422Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting LabWired Simulator
[2m2026-10-18T14:39:50.035547Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../chips/onboarding/ambiq-apollo4.yaml
[2m2026-10-18T14:39:50.039591Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'ambiqapollo4_gpio' (peripheral 'gpio') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:39:50.039826Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Loading firmware: "/home/andrii/Projects/labwired-renode/core/tests/fixtures/uart-ok-thumbv7m.elf"
[2m2026-10-18T14:39:50.039902Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x401
[2m2026-10-18T14:39:50.039913Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Firmware Loaded Successfully!
[2m2026-10-18T14:39:50.039918Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Entry Point: 0x401
[2m2026-10-18T14:39:50.040936Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting Simulation (ARM Cortex-M)...
[2m2026-10-18T14:39:50.040949Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Initial PC: 0x400, SP: 0x20020000
[2m2026-10-18T14:39:50.040959Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Running for 20000 steps...
[2m2026-10-18T14:39:50.047935Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Progress: 10000 steps, current IPS: 1249300.55
[2m2026-10-18T14:39:50.057835Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Simulation loop finished.
[2m2026-10-18T14:39:50.057854Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Final PC: 0x466
[2m2026-10-18T14:39:50.057859Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Instructions: 19999
[2m2026-10-18T14:39:50.057863Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Cycles: 20002
[2m2026-10-18T14:39:50.057867Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Average IPS: 1114777.55
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `bootrom_logger` | `0x07fffffc` | `ambiqapollo4_bootromlogger` |
| `adc` | `0x40038000` | `stub` |
| `gpio` | `0x40010000` | `ambiqapollo4_gpio` |
| `iom0` | `0x40050000` | `stub` |
| `iom1` | `0x40051000` | `stub` |
| `iom2` | `0x40052000` | `stub` |
| `iom3` | `0x40053000` | `stub` |
| `iom4` | `0x40054000` | `stub` |
| `iom5` | `0x40055000` | `stub` |
| `iom6` | `0x40056000` | `stub` |
| `iom7` | `0x40057000` | `stub` |
| `rtc` | `0x40004800` | `stub` |
| `timer` | `0x40008000` | `ambiqapollo4_timer` |
| `stimer` | `0x40008800` | `ambiqapollo4_systemtimer` |
| `uart0` | `0x4001c000` | `pl011` |
| `uart1` | `0x4001d000` | `pl011` |
| `uart2` | `0x4001e000` | `pl011` |
| `uart3` | `0x4001f000` | `pl011` |
| `pwrctrl` | `0x40021000` | `stub` |
| `security` | `0x40030000` | `stub` |
| `wdt` | `0x40024000` | `stub` |
| `cpu_complex` | `0x48000000` | `pythonperipheral` |

**Total Peripherals:** 22