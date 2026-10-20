# Validation Report: sam4s

**Architecture:** ARMv7E-M.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
[2m2026-10-18T14:40:21.909023Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting LabWired Simulator
[2m2026-10-18T14:40:21.909527Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../chips/onboarding/sam4s.yaml
[2m2026-10-18T14:40:21.920131Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'sam4s_pio' (peripheral 'pioA') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:40:21.920177Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'sam4s_pio' (peripheral 'pioB') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:40:21.920188Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'sam4s_pio' (peripheral 'pioC') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:40:21.920320Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Loading firmware: "/home/andrii/Projects/labwired-renode/core/tests/fixtures/uart-ok-thumbv7m.elf"
[2m2026-10-18T14:40:21.920421Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x401
[2m2026-10-18T14:40:21.920451Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Firmware Loaded Successfully!
[2m2026-10-18T14:40:21.920456Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Entry Point: 0x401
[2m2026-10-18T14:40:21.927366Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting Simulation (ARM Cortex-M)...
[2m2026-10-18T14:40:21.927388Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Initial PC: 0x400, SP: 0x20020000
[2m2026-10-18T14:40:21.927398Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Running for 20000 steps...
[2m2026-10-18T14:40:21.934527Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Progress: 10000 steps, current IPS: 711095.28
[2m2026-10-18T14:40:21.941331Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Simulation loop finished.
[2m2026-10-18T14:40:21.941353Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Final PC: 0x466
[2m2026-10-18T14:40:21.941361Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Instructions: 19999
[2m2026-10-18T14:40:21.941364Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Cycles: 20002
[2m2026-10-18T14:40:21.941368Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Average IPS: 956526.56
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `dwt` | `0xe0001000` | `dwt` |
| `sram_bb` | `0x22000000` | `bitbanding` |
| `tc0_1_2` | `0x40010000` | `sam_tc` |
| `tc3_4_5` | `0x40014000` | `sam_tc` |
| `usart0` | `0x40024000` | `sam_usart` |
| `usart1` | `0x40028000` | `sam_usart` |
| `adc` | `0x40038000` | `stub` |
| `dacc` | `0x4003c000` | `stub` |
| `PMC_SR` | `0x400e0468` | `pythonperipheral` |
| `uart0` | `0x400e0600` | `sam_usart` |
| `uart1` | `0x400e0800` | `sam_usart` |
| `eefc` | `0x400e0a00` | `stub` |
| `rstc` | `0x400e1400` | `stub` |
| `wdt` | `0x400e1450` | `stub` |
| `peripheral_bb` | `0x42000000` | `bitbanding` |
| `crc` | `0x40044000` | `stub` |
| `pioA` | `0x400e0e00` | `sam4s_pio` |
| `pioB` | `0x400e1000` | `sam4s_pio` |
| `pioC` | `0x400e1200` | `sam4s_pio` |
| `spi` | `0x40008000` | `stub` |
| `twi1` | `0x4001c000` | `stub` |
| `twi0` | `0x40018000` | `stub` |

**Total Peripherals:** 22