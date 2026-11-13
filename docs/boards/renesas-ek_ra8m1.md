# Validation Report: renesas-ek_ra8m1

> **Superseded.** This is the auto-generated report for the staged
> `onboarding/renesas-ek_ra8m1` descriptor (a generic UART fixture on a
> placeholder memory map: flash at `0x0800_0000` and RAM at `0x3000_0000`,
> not the RA8M1's code flash at `0x0200_0000` and SRAM at `0x2200_0000`). The
> promoted RA8M1 part is [`r7fa8m1ah`](r7fa8m1ah.md), and the board package is
> **[EK-RA8M1](ek-ra8m1.md)**.

**Architecture:** ARM Cortex-M85.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
[2m2026-10-18T14:40:20.654040Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting LabWired Simulator
[2m2026-10-18T14:40:20.654681Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../chips/onboarding/renesas-ek_ra8m1.yaml
[2m2026-10-18T14:40:20.657896Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'renesasra_gpiomisc' (peripheral 'portMisc') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:40:20.658117Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Loading firmware: "/home/andrii/Projects/labwired-renode/core/tests/fixtures/uart-ok-thumbv7m.elf"
[2m2026-10-18T14:40:20.658906Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x401
[2m2026-10-18T14:40:20.659286Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Firmware Loaded Successfully!
[2m2026-10-18T14:40:20.659333Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Entry Point: 0x401
[2m2026-10-18T14:40:20.660438Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting Simulation (ARM Cortex-M)...
[2m2026-10-18T14:40:20.662447Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Initial PC: 0x400, SP: 0x20020000
[2m2026-10-18T14:40:20.662515Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Running for 20000 steps...
[2m2026-10-18T14:40:20.662637Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Simulation Error at step 2: Memory access violation at 0x2001ffe0
[2m2026-10-18T14:40:20.662932Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Simulation loop finished.
[2m2026-10-18T14:40:20.662975Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Final PC: 0x45c
[2m2026-10-18T14:40:20.663002Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Instructions: 2
[2m2026-10-18T14:40:20.664291Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Cycles: 2
[2m2026-10-18T14:40:20.664362Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Average IPS: 400.13
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `led_blue` | `0x00000000` | `led` |
| `led_green` | `0x0000000e` | `led` |
| `led_red` | `0x0000000d` | `led` |
| `button_s2` | `0x00000008` | `button` |
| `button_s1` | `0x00000009` | `button` |
| `gpt` | `0x40322000` | `renesasra_gpt` |
| `gpt_ns` | `0x50322000` | `renesasra_gpt` |
| `agt0` | `0x40221000` | `renesasra_agt` |
| `agt1` | `0x40221100` | `renesasra_agt` |
| `sci0` | `0x40358000` | `renesasra8m1_sci` |
| `sci1` | `0x40358100` | `renesasra8m1_sci` |
| `sci2` | `0x40358200` | `renesasra8m1_sci` |
| `sci3` | `0x40358300` | `renesasra8m1_sci` |
| `sci4` | `0x40358400` | `renesasra8m1_sci` |
| `sci9` | `0x40358900` | `renesasra8m1_sci` |
| `portMisc` | `0x40400d00` | `renesasra_gpiomisc` |
| `SYSC_SCICKCR` | `0x4001e055` | `pythonperipheral` |
| `iic0` | `0x4025e000` | `stub` |
| `iic1` | `0x4025e100` | `stub` |

**Total Peripherals:** 19