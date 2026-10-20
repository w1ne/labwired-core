# Validation Report: nxp-mimx8ml

**Architecture:** ARM multicore cluster.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
[2m2026-10-18T14:40:13.722964Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting LabWired Simulator
[2m2026-10-18T14:40:13.723522Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../chips/onboarding/nxp-mimx8ml.yaml
[2m2026-10-18T14:40:18.236452Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'nxp_imx_gpio' (peripheral 'gpio1') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:40:18.236571Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'nxp_imx_gpio' (peripheral 'gpio2') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:40:18.236613Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'nxp_imx_gpio' (peripheral 'gpio3') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:40:18.236650Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'nxp_imx_gpio' (peripheral 'gpio4') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:40:18.236687Z[0m [33m WARN[0m [2mlabwired_core::bus::profiles[0m[2m:[0m GPIO type 'nxp_imx_gpio' (peripheral 'gpio5') has no dedicated register model; falling back to the STM32F1 placeholder layout. This is a known-incomplete onboarding model — set `config.profile` or add a real model for correct behaviour.
[2m2026-10-18T14:40:18.236953Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Loading firmware: "/home/andrii/Projects/labwired-renode/core/tests/fixtures/uart-ok-thumbv7m.elf"
[2m2026-10-18T14:40:18.237073Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x401
[2m2026-10-18T14:40:18.237114Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Firmware Loaded Successfully!
[2m2026-10-18T14:40:18.237141Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Entry Point: 0x401
[2m2026-10-18T14:40:18.238620Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting Simulation (ARM Cortex-M)...
[2m2026-10-18T14:40:18.238679Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Initial PC: 0x400, SP: 0x20020000
[2m2026-10-18T14:40:18.238709Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Running for 20000 steps...
[2m2026-10-18T14:40:18.248857Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Progress: 10000 steps, current IPS: 855683.84
[2m2026-10-18T14:40:18.259322Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Simulation loop finished.
[2m2026-10-18T14:40:18.259406Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Final PC: 0x454
[2m2026-10-18T14:40:18.259415Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Instructions: 20000
[2m2026-10-18T14:40:18.259420Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Cycles: 20004
[2m2026-10-18T14:40:18.259425Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Average IPS: 898531.65
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
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

**Total Peripherals:** 36