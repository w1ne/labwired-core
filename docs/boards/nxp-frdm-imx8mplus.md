# Validation Report: nxp-frdm-imx8mplus

**Architecture:** ARM multicore cluster.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
[2m2026-10-18T14:40:08.145579Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting LabWired Simulator
[2m2026-10-18T14:40:08.147555Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../chips/onboarding/nxp-frdm-imx8mplus.yaml
[2m2026-10-18T14:40:12.760650Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'nxp_imx_gpio' (peripheral 'gpio1') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:40:12.760759Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'nxp_imx_gpio' (peripheral 'gpio2') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:40:12.760798Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'nxp_imx_gpio' (peripheral 'gpio3') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:40:12.760830Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'nxp_imx_gpio' (peripheral 'gpio4') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:40:12.760860Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'nxp_imx_gpio' (peripheral 'gpio5') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:40:12.761125Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Loading firmware: "/home/andrii/Projects/labwired-renode/core/tests/fixtures/uart-ok-thumbv7m.elf"
[2m2026-10-18T14:40:12.761224Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x401
[2m2026-10-18T14:40:12.761258Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Firmware Loaded Successfully!
[2m2026-10-18T14:40:12.761282Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Entry Point: 0x401
[2m2026-10-18T14:40:12.768489Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting Simulation (ARM Cortex-M)...
[2m2026-10-18T14:40:12.768508Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Initial PC: 0x400, SP: 0x20020000
[2m2026-10-18T14:40:12.768514Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Running for 20000 steps...
[2m2026-10-18T14:40:12.783387Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Progress: 10000 steps, current IPS: 452991.99
[2m2026-10-18T14:40:12.796834Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Simulation loop finished.
[2m2026-10-18T14:40:12.796927Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Final PC: 0x454
[2m2026-10-18T14:40:12.796966Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Instructions: 20000
[2m2026-10-18T14:40:12.797000Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Cycles: 20004
[2m2026-10-18T14:40:12.797031Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Average IPS: 559833.68
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `phy_mac` | `0x00000001` | `ethernetphysicallayer` |
| `phy_qos` | `0x00000000` | `ethernetphysicallayer` |
| `pmic` | `0x00000025` | `dummyi2cslave` |
| `gpio_exp1` | `0x00000020` | `dummyi2cslave` |
| `gpio_exp2` | `0x00000021` | `dummyi2cslave` |
| `codec` | `0x0000001a` | `dummyi2cslave` |
| `gpt1` | `0x302d0000` | `imx_gptimer` |
| `gpt2` | `0x302e0000` | `imx_gptimer` |
| `gpt3` | `0x302f0000` | `imx_gptimer` |
| `gpt4` | `0x30700000` | `imx_gptimer` |
| `gpt5` | `0x306f0000` | `imx_gptimer` |
| `gpt6` | `0x306e0000` | `imx_gptimer` |
| `gpio1` | `0x30200000` | `nxp_imx_gpio` |
| `gpio2` | `0x30210000` | `nxp_imx_gpio` |
| `gpio3` | `0x30220000` | `nxp_imx_gpio` |
| `gpio4` | `0x30230000` | `nxp_imx_gpio` |
| `gpio5` | `0x30240000` | `nxp_imx_gpio` |
| `uart1` | `0x30860000` | `mxcuart` |
| `uart2` | `0x30890000` | `mxcuart` |
| `uart3` | `0x30880000` | `mxcuart` |
| `uart4` | `0x30a60000` | `mxcuart` |
| `usdhc1` | `0x30b40000` | `nxp_usdhc` |
| `usdhc2` | `0x30b50000` | `nxp_usdhc` |
| `usdhc3` | `0x30b60000` | `nxp_usdhc` |
| `wdog1` | `0x30280000` | `nxp_imx_watchdog` |
| `wdog2` | `0x30290000` | `nxp_imx_watchdog` |
| `wdog3` | `0x302a0000` | `nxp_imx_watchdog` |
| `src` | `0x30390000` | `nxp_imx8mp_systemresetcontroller` |
| `flexcan1` | `0x308c0000` | `nxp_flexcan` |
| `flexcan2` | `0x308d0000` | `nxp_flexcan` |
| `eth_mac` | `0x30be0000` | `k6xf_ethernet` |
| `eth_qos` | `0x30bf0000` | `synopsysdwcethernetqualityofservice` |
| `i2c1` | `0x30a20000` | `stub` |
| `i2c2` | `0x30a30000` | `stub` |
| `i2c3` | `0x30a40000` | `stub` |
| `i2c4` | `0x30a50000` | `stub` |
| `i2c5` | `0x30ad0000` | `stub` |
| `i2c6` | `0x30ae0000` | `stub` |
| `apis1_gpc` | `0x303a0000` | `pythonperipheral` |
| `ecspi1` | `0x30820000` | `stub` |
| `ecspi2` | `0x30830000` | `stub` |
| `ecspi3` | `0x30840000` | `stub` |

**Total Peripherals:** 42