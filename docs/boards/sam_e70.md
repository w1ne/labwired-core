# Validation Report: sam_e70

> **Superseded.** This is the auto-generated report for the staged
> `onboarding/sam_e70` descriptor (a generic UART fixture on a placeholder
> memory map: 256 MiB of flash at `0x0800_0000` and 256 MiB of RAM, copied
> from the Renode platform, not the SAM E70's 2 MiB flash at `0x0040_0000`).
> The promoted SAM E70 part is [`same70q21b`](same70q21b.md), and the board
> package is **[SAM E70 Xplained](sam-e70-xplained.md)**. The staged
> descriptor stays only because `crates/core/src/bus/known_stubs.rs` still
> cites it.

**Architecture:** ARMv7E-M.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
[2m2026-10-18T14:40:22.484351Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting LabWired Simulator
[2m2026-10-18T14:40:22.484910Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../chips/onboarding/sam_e70.yaml
[2m2026-10-18T14:40:22.978047Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Loading firmware: "/home/andrii/Projects/labwired-renode/core/tests/fixtures/uart-ok-thumbv7m.elf"
[2m2026-10-18T14:40:22.978215Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x401
[2m2026-10-18T14:40:22.978267Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Firmware Loaded Successfully!
[2m2026-10-18T14:40:22.978303Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Entry Point: 0x401
[2m2026-10-18T14:40:22.979007Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting Simulation (ARM Cortex-M)...
[2m2026-10-18T14:40:22.982451Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Initial PC: 0x400, SP: 0x20020000
[2m2026-10-18T14:40:22.982471Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Running for 20000 steps...
[2m2026-10-18T14:40:22.986616Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Progress: 10000 steps, current IPS: 1207770.89
[2m2026-10-18T14:40:22.997295Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Simulation loop finished.
[2m2026-10-18T14:40:23.000447Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Final PC: 0x466
[2m2026-10-18T14:40:23.000535Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Instructions: 19999
[2m2026-10-18T14:40:23.000578Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Cycles: 20002
[2m2026-10-18T14:40:23.000613Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Average IPS: 897696.41
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `usart0` | `0x40024000` | `sam_usart` |
| `usart1` | `0x40028000` | `sam_usart` |
| `usart2` | `0x4002c000` | `sam_usart` |
| `gem` | `0x40050000` | `cadencegem` |
| `phy` | `0x00000000` | `ethernetphysicallayer` |
| `trng` | `0x40070000` | `sam_trng` |
| `PMC_SR` | `0x400e0668` | `pythonperipheral` |

**Total Peripherals:** 7