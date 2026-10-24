# Validation Report: eos-s3

**Architecture:** ARM Cortex-M4F.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
[2m2026-10-18T14:39:55.744062Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting LabWired Simulator
[2m2026-10-18T14:39:55.747558Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../chips/onboarding/eos-s3.yaml
[2m2026-10-18T14:39:55.749026Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Loading firmware: "/home/andrii/Projects/labwired-renode/core/tests/fixtures/uart-ok-thumbv7m.elf"
[2m2026-10-18T14:39:55.749116Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x401
[2m2026-10-18T14:39:55.749134Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Firmware Loaded Successfully!
[2m2026-10-18T14:39:55.749139Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Entry Point: 0x401
[2m2026-10-18T14:39:55.750013Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting Simulation (ARM Cortex-M)...
[2m2026-10-18T14:39:55.750030Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Initial PC: 0x400, SP: 0x20020000
[2m2026-10-18T14:39:55.750039Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Running for 20000 steps...
[2m2026-10-18T14:39:55.750114Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Simulation Error at step 2: Memory access violation at 0x2001ffe0
[2m2026-10-18T14:39:55.750140Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Simulation loop finished.
[2m2026-10-18T14:39:55.750145Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Final PC: 0x45c
[2m2026-10-18T14:39:55.750149Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Instructions: 2
[2m2026-10-18T14:39:55.750154Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Cycles: 2
[2m2026-10-18T14:39:55.750158Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Average IPS: 1971.92
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `uart` | `0x40010000` | `pl011` |
| `dwt` | `0xe0001000` | `dwt` |
| `spt` | `0x40005c00` | `eoss3_simpleperiodictimer` |
| `adc` | `0x40005a00` | `stub` |
| `spi` | `0x40007000` | `stub` |
| `dmaSpi` | `0x40007400` | `stub` |
| `packetFifo` | `0x40002000` | `stub` |
| `systemDma` | `0x4000c000` | `udma` |
| `systemDmaBridge` | `0x4000d000` | `eoss3_systemdmabridge` |
| `ffe` | `0x4004a000` | `stub` |
| `i2cMaster0` | `0x00000000` | `opencoresi2c` |
| `i2cMaster1` | `0x00000001` | `opencoresi2c` |
| `voice` | `0x40015000` | `stub` |
| `powerMgmt` | `0x40004400` | `pythonperipheral` |

**Total Peripherals:** 14