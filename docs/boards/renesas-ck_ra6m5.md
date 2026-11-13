# Validation Report: renesas-ck_ra6m5

> **Superseded.** This is the auto-generated report for the staged
> `onboarding/renesas-ck_ra6m5` descriptor (a generic UART fixture on a placeholder
> memory map: its code flash at `0x0800_0000` is the RA6M5 data-flash address,
> not the 2 MiB code flash at `0x0`). The promoted RA6M5 part is
> [`r7fa6m5bh`](r7fa6m5bh.md), and the board package is
> **[EK-RA6M5](ek-ra6m5.md)**. The staged descriptor stays only because
> `crates/core/src/bus/known_stubs.rs` still cites it.

**Architecture:** ARM Cortex-M33.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
[2m2026-10-18T14:40:20.311205Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting LabWired Simulator
[2m2026-10-18T14:40:20.317562Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../chips/onboarding/renesas-ck_ra6m5.yaml
[2m2026-10-18T14:40:20.323773Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'renesasra_gpiomisc' (peripheral 'portMisc') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:40:20.323943Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Loading firmware: "/home/andrii/Projects/labwired-renode/core/tests/fixtures/uart-ok-thumbv7m.elf"
[2m2026-10-18T14:40:20.324035Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x401
[2m2026-10-18T14:40:20.324045Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Firmware Loaded Successfully!
[2m2026-10-18T14:40:20.324049Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Entry Point: 0x401
[2m2026-10-18T14:40:20.325335Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting Simulation (ARM Cortex-M)...
[2m2026-10-18T14:40:20.325345Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Initial PC: 0x400, SP: 0x20020000
[2m2026-10-18T14:40:20.325351Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Running for 20000 steps...
[2m2026-10-18T14:40:20.335105Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Progress: 10000 steps, current IPS: 905134.81
[2m2026-10-18T14:40:20.348470Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Simulation loop finished.
[2m2026-10-18T14:40:20.348563Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Final PC: 0x466
[2m2026-10-18T14:40:20.348596Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Instructions: 19999
[2m2026-10-18T14:40:20.348624Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Cycles: 20002
[2m2026-10-18T14:40:20.348650Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Average IPS: 813076.68
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `led_red` | `0x0000000a` | `led` |
| `led_green` | `0x00000009` | `led` |
| `led_blue` | `0x00000001` | `led` |
| `button` | `0x00000004` | `button` |
| `hs3001` | `0x00000044` | `hs3001` |
| `iaq` | `0x00000032` | `zmod4xxx` |
| `oaq` | `0x00000033` | `zmod4xxx` |
| `barometer` | `0x00000063` | `icp_101xx` |
| `icm` | `0x00000068` | `icm20948` |
| `magnetometer` | `0x0000000c` | `ak09916` |
| `sysc_oscsf` | `0x4001e03c` | `pythonperipheral` |
| `gpt` | `0x40169000` | `renesasra_gpt` |
| `agt0` | `0x400e8000` | `renesasra_agt` |
| `agt1` | `0x400e8100` | `renesasra_agt` |
| `agt2` | `0x400e8200` | `renesasra_agt` |
| `agt3` | `0x400e8300` | `renesasra_agt` |
| `agt4` | `0x400e8400` | `renesasra_agt` |
| `agt5` | `0x400e8500` | `renesasra_agt` |
| `sci0` | `0x40118000` | `renesasra6m5_sci` |
| `sci1` | `0x40118100` | `renesasra6m5_sci` |
| `sci2` | `0x40118200` | `renesasra6m5_sci` |
| `sci3` | `0x40118300` | `renesasra6m5_sci` |
| `sci4` | `0x40118400` | `renesasra6m5_sci` |
| `sci5` | `0x40118500` | `renesasra6m5_sci` |
| `sci6` | `0x40118600` | `renesasra6m5_sci` |
| `sci7` | `0x40118700` | `renesasra6m5_sci` |
| `sci8` | `0x40118800` | `renesasra6m5_sci` |
| `sci9` | `0x40118900` | `renesasra6m5_sci` |
| `portMisc` | `0x40080d00` | `renesasra_gpiomisc` |
| `iic0` | `0x4009f000` | `stub` |
| `iic1` | `0x4009f100` | `stub` |
| `iic2` | `0x4009f200` | `stub` |

**Total Peripherals:** 32