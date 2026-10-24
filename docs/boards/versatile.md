# Validation Report: versatile

**Architecture:** ARMv7-A.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
[2m2026-10-18T14:40:36.886074Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting LabWired Simulator
[2m2026-10-18T14:40:36.886466Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../chips/onboarding/versatile.yaml
[2m2026-10-18T14:40:37.151231Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Loading firmware: "/home/andrii/Projects/labwired-renode/core/tests/fixtures/uart-ok-thumbv7m.elf"
[2m2026-10-18T14:40:37.151317Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x401
[2m2026-10-18T14:40:37.151326Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Firmware Loaded Successfully!
[2m2026-10-18T14:40:37.151330Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Entry Point: 0x401
[2m2026-10-18T14:40:37.152206Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting Simulation (ARM Cortex-M)...
[2m2026-10-18T14:40:37.152214Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Initial PC: 0x400, SP: 0x20020000
[2m2026-10-18T14:40:37.152223Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Running for 20000 steps...
[2m2026-10-18T14:40:37.156605Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Progress: 10000 steps, current IPS: 1898063.37
[2m2026-10-18T14:40:37.160686Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Simulation loop finished.
[2m2026-10-18T14:40:37.160764Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Final PC: 0x466
[2m2026-10-18T14:40:37.160793Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Instructions: 19999
[2m2026-10-18T14:40:37.160819Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Cycles: 20002
[2m2026-10-18T14:40:37.160844Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Average IPS: 2103035.38
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `usbEhci` | `0x00000001` | `isp1761` |
| `usbHub` | `0x00000001` | `usbhub` |
| `lcd` | `0x10120000` | `pl110` |
| `controller` | `0x20000000` | `emulatorcontroller` |
| `usbMouse` | `0x00000002` | `usbmouse` |
| `usbKeyboard` | `0x00000001` | `usbkeyboard` |
| `uart0` | `0x101f1000` | `pl011` |
| `uart1` | `0x101f2000` | `pl011` |
| `sysctl` | `0x10000000` | `armsysctl` |
| `timer1` | `0x101e2000` | `arm_sp804_timer` |
| `timer2` | `0x101e3000` | `arm_sp804_timer` |
| `smc91x` | `0x10010000` | `smc91x` |
| `kmi0` | `0x10006000` | `pl050` |
| `kmi1` | `0x10007000` | `pl050` |

**Total Peripherals:** 14