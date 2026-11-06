# Validation Report: renesas-rz_t2m_rsk

**Architecture:** Dual ARM Cortex-R52.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
[2m2026-10-18T14:40:21.157763Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting LabWired Simulator
[2m2026-10-18T14:40:21.161674Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../chips/onboarding/renesas-rz_t2m_rsk.yaml
[2m2026-10-18T14:40:21.308749Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Loading firmware: "/home/andrii/Projects/labwired-renode/core/tests/fixtures/uart-ok-thumbv7m.elf"
[2m2026-10-18T14:40:21.308858Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x401
[2m2026-10-18T14:40:21.308869Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Firmware Loaded Successfully!
[2m2026-10-18T14:40:21.308874Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Entry Point: 0x401
[2m2026-10-18T14:40:21.313228Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting Simulation (ARM Cortex-M)...
[2m2026-10-18T14:40:21.313302Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Initial PC: 0x400, SP: 0x20020000
[2m2026-10-18T14:40:21.313339Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Running for 20000 steps...
[2m2026-10-18T14:40:21.313467Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Simulation Error at step 2: Memory access violation at 0x2001ffe0
[2m2026-10-18T14:40:21.313519Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Simulation loop finished.
[2m2026-10-18T14:40:21.313545Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Final PC: 0x45c
[2m2026-10-18T14:40:21.313569Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Instructions: 2
[2m2026-10-18T14:40:21.313592Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Cycles: 2
[2m2026-10-18T14:40:21.313613Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Average IPS: 422.41
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `timer` | `0x0000000f` | `arm_generictimer` |
| `sci0` | `0x80001000` | `renesas_sci` |
| `sci1` | `0x80001400` | `renesas_sci` |
| `sci2` | `0x80001800` | `renesas_sci` |
| `sci3` | `0x80001c00` | `renesas_sci` |
| `sci4` | `0x80002000` | `renesas_sci` |
| `sci5` | `0x81001000` | `renesas_sci` |

**Total Peripherals:** 7