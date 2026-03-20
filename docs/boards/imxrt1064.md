# Validation Report: imxrt1064

**Architecture:** ARMv7E-M.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
Simulation completed successfully with no warnings.
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
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

**Total Peripherals:** 36