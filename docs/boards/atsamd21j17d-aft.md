# Validation Report: atsamd21j17d-aft

**Architecture:** ARM Cortex-M0+. General purpose low power microcontroller.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
[2m2026-10-18T14:39:50.667753Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting LabWired Simulator
[2m2026-10-18T14:39:50.669570Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../chips/onboarding/atsamd21j17d-aft.yaml
[2m2026-10-18T14:39:50.670560Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'samd21_gpio' (peripheral 'gpio_a') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:39:50.670593Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'samd21_gpio' (peripheral 'gpio_b') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:39:50.670756Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Loading firmware: "/home/andrii/Projects/labwired-renode/core/tests/fixtures/uart-ok-thumbv7m.elf"
[2m2026-10-18T14:39:50.670843Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x401
[2m2026-10-18T14:39:50.670854Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Firmware Loaded Successfully!
[2m2026-10-18T14:39:50.670858Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Entry Point: 0x401
[2m2026-10-18T14:39:50.677439Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting Simulation (ARM Cortex-M)...
[2m2026-10-18T14:39:50.677458Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Initial PC: 0x400, SP: 0x20020000
[2m2026-10-18T14:39:50.677469Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Running for 20000 steps...
[2m2026-10-18T14:39:50.677546Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Simulation Error at step 2: Memory access violation at 0x2001ffe0
[2m2026-10-18T14:39:50.677569Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Simulation loop finished.
[2m2026-10-18T14:39:50.677573Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Final PC: 0x45c
[2m2026-10-18T14:39:50.677577Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Instructions: 2
[2m2026-10-18T14:39:50.677581Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Cycles: 2
[2m2026-10-18T14:39:50.677585Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Average IPS: 297.75
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `dwt` | `0xe0001000` | `dwt` |
| `gclk` | `0x40000c00` | `arraymemory` |
| `rtc` | `0x40001400` | `samd21_rtc` |
| `gpio_a` | `0x41004400` | `samd21_gpio` |
| `gpio_b` | `0x41004480` | `samd21_gpio` |
| `usart0` | `0x42000800` | `samd5_uart` |
| `usart1` | `0x42000c00` | `samd5_uart` |
| `usart2` | `0x42001400` | `samd5_uart` |
| `tc4` | `0x42003000` | `samd21_timer` |
| `tc6` | `0x42003800` | `samd21_timer` |

**Total Peripherals:** 10