# Validation Report: zynqmp-zcu102-revA

**Architecture:** Quad ARM Cortex-A53 / Dual Cortex-R5F.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
[2m2026-10-18T14:40:42.132326Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting LabWired Simulator
[2m2026-10-18T14:40:42.134664Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../chips/onboarding/zynqmp-zcu102-revA.yaml
[2m2026-10-18T14:40:43.095469Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'xilinxgpiops' (peripheral 'gpio') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:40:43.095655Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Loading firmware: "/home/andrii/Projects/labwired-renode/core/tests/fixtures/uart-ok-thumbv7m.elf"
[2m2026-10-18T14:40:43.095753Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x401
[2m2026-10-18T14:40:43.095766Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Firmware Loaded Successfully!
[2m2026-10-18T14:40:43.095770Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Entry Point: 0x401
[2m2026-10-18T14:40:43.097117Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting Simulation (ARM Cortex-M)...
[2m2026-10-18T14:40:43.100447Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Initial PC: 0x400, SP: 0x20020000
[2m2026-10-18T14:40:43.100466Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Running for 20000 steps...
[2m2026-10-18T14:40:43.104845Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Progress: 10000 steps, current IPS: 1102954.84
[2m2026-10-18T14:40:43.116064Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Simulation loop finished.
[2m2026-10-18T14:40:43.116154Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Final PC: 0x454
[2m2026-10-18T14:40:43.116191Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Instructions: 20000
[2m2026-10-18T14:40:43.116221Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Cycles: 20004
[2m2026-10-18T14:40:43.116250Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Average IPS: 976780.18
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `phy3` | `0x00000015` | `ethernetphysicallayer` |
| `i2c_mux` | `0x00000075` | `pca9548` |
| `tca6416_u97` | `0x00000020` | `tca6416` |
| `tca6416_u61` | `0x00000021` | `tca6416` |
| `apu0Timer` | `0x0000000f` | `arm_generictimer` |
| `apu1Timer` | `0x0000000f` | `arm_generictimer` |
| `apu2Timer` | `0x0000000f` | `arm_generictimer` |
| `apu3Timer` | `0x0000000f` | `arm_generictimer` |
| `uart0` | `0xff000000` | `cadence_uart` |
| `uart1` | `0xff010000` | `cadence_uart` |
| `i2c0` | `0xff020000` | `stub` |
| `i2c1` | `0xff030000` | `stub` |
| `ttc0` | `0xff110000` | `cadence_ttc` |
| `ttc1` | `0xff120000` | `cadence_ttc` |
| `ttc2` | `0xff130000` | `cadence_ttc` |
| `ttc3` | `0xff140000` | `cadence_ttc` |
| `gem0` | `0xff0b0000` | `cadencegem` |
| `gem1` | `0xff0c0000` | `cadencegem` |
| `gem2` | `0xff0d0000` | `cadencegem` |
| `gem3` | `0xff0e0000` | `cadencegem` |
| `phy` | `0x00000000` | `ethernetphysicallayer` |
| `gpio` | `0xff0a0000` | `xilinxgpiops` |
| `ipi` | `0xff300000` | `stub` |
| `rtc` | `0xffa60000` | `stub` |
| `platformManagementUnit` | `0x0000000a` | `stub` |
| `L1_PLL_STATUS_READ_1_SERDES_REGISTER` | `0xfd4063e4` | `arraymemory` |
| `L2_PLL_STATUS_READ_1_SERDES_REGISTER` | `0xfd40a3e4` | `arraymemory` |
| `L3_PLL_STATUS_READ_1_SERDES_REGISTER` | `0xfd40e3e4` | `arraymemory` |
| `PS_CTRL_STATUS_AMS_REGISTER` | `0xffa50040` | `pythonperipheral` |
| `qspi` | `0xff0f0000` | `stub` |

**Total Peripherals:** 30