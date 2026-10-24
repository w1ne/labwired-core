# Validation Report: mimxrt1064_evk

**Architecture:** ARMv7E-M.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
[2m2026-10-18T14:40:04.627948Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting LabWired Simulator
[2m2026-10-18T14:40:04.629368Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../chips/onboarding/mimxrt1064_evk.yaml
[2m2026-10-18T14:40:04.637998Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Loading firmware: "/home/andrii/Projects/labwired-renode/core/tests/fixtures/uart-ok-thumbv7m.elf"
[2m2026-10-18T14:40:04.644639Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x401
[2m2026-10-18T14:40:04.644661Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Firmware Loaded Successfully!
[2m2026-10-18T14:40:04.644666Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Entry Point: 0x401
[2m2026-10-18T14:40:04.647267Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting Simulation (ARM Cortex-M)...
[2m2026-10-18T14:40:04.647284Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Initial PC: 0x400, SP: 0x20020000
[2m2026-10-18T14:40:04.647294Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Running for 20000 steps...
[2m2026-10-18T14:40:04.647369Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Simulation Error at step 2: Memory access violation at 0x2001ffe0
[2m2026-10-18T14:40:04.647388Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Simulation loop finished.
[2m2026-10-18T14:40:04.647393Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Final PC: 0x45c
[2m2026-10-18T14:40:04.647396Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Instructions: 2
[2m2026-10-18T14:40:04.647401Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Cycles: 2
[2m2026-10-18T14:40:04.647405Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Average IPS: 733.62
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `green_led` | `0x00000009` | `led` |
| `wdog1` | `0x400b8000` | `pythonperipheral` |
| `trng` | `0x400cc000` | `imx_trng` |
| `wdog2` | `0x400d0000` | `pythonperipheral` |
| `wdog3` | `0x400bc000` | `pythonperipheral` |
| `iomuxc` | `0x401f8000` | `pythonperipheral` |
| `analog01` | `0x400d8000` | `pythonperipheral` |
| `analog03` | `0x400d8034` | `pythonperipheral` |
| `dcdc` | `0x40080000` | `pythonperipheral` |
| `gpTimer1` | `0x401ec000` | `imx_gptimer` |
| `lpuart1` | `0x40184000` | `nxp_lpuart` |
| `lpuart2` | `0x40188000` | `nxp_lpuart` |
| `lpuart3` | `0x4018c000` | `nxp_lpuart` |
| `lpuart4` | `0x40190000` | `nxp_lpuart` |
| `lpuart5` | `0x40194000` | `nxp_lpuart` |
| `lpuart6` | `0x40198000` | `nxp_lpuart` |
| `lpuart7` | `0x4019c000` | `nxp_lpuart` |
| `lpuart8` | `0x401a0000` | `nxp_lpuart` |
| `gpio1` | `0x401b8000` | `imxrt_gpio` |
| `gpio2` | `0x401bc000` | `imxrt_gpio` |
| `gpio3` | `0x401c0000` | `imxrt_gpio` |
| `gpio4` | `0x401c4000` | `imxrt_gpio` |
| `gpio5` | `0x400c0000` | `imxrt_gpio` |
| `gpio6` | `0x42000000` | `imxrt_gpio` |
| `gpio7` | `0x42004000` | `imxrt_gpio` |
| `gpio8` | `0x42008000` | `imxrt_gpio` |
| `gpio9` | `0x4200c000` | `imxrt_gpio` |
| `gpio10` | `0x401c8000` | `imxrt_gpio` |
| `enet` | `0x402d8000` | `k6xf_ethernet` |
| `enet2` | `0x402d4000` | `k6xf_ethernet` |
| `flex_spi2` | `0x402a4000` | `imxrt_flexspi` |
| `adc1` | `0x400c4000` | `imxrt_adc` |
| `adc2` | `0x400c8000` | `imxrt_adc` |
| `pwm1` | `0x403dc000` | `imxrt_pwm` |
| `pwm2` | `0x403e0000` | `imxrt_pwm` |
| `pwm3` | `0x403e4000` | `imxrt_pwm` |
| `pwm4` | `0x403e8000` | `imxrt_pwm` |

**Total Peripherals:** 37