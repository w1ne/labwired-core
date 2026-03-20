# Validation Report: nxp-s32k388evb

**Architecture:** ARMv7E-M.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
Simulation completed successfully with no warnings.
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `userButton0` | `0x00000001` | `button` |
| `userButton1` | `0x0000001c` | `button` |
| `led2_red` | `0x000000b1` | `led` |
| `led2_green` | `0x000000b2` | `led` |
| `led2_blue` | `0x000000b3` | `led` |
| `led1_red` | `0x000000b6` | `led` |
| `led1_green` | `0x000000b7` | `led` |
| `led1_blue` | `0x000000b8` | `led` |
| `dwt` | `0xe0001000` | `dwt` |
| `mscm` | `0x40260000` | `s32k3xx_miscellaneoussystemcontrolmodule` |
| `siul2` | `0x40290000` | `s32k3xx_systemintegrationunitlite2` |
| `mc_me` | `0x402dc000` | `s32kxx_modeentrymodule` |
| `swt0` | `0x40270000` | `s32k3xx_softwarewatchdogtimer` |
| `swt1` | `0x4046c000` | `s32k3xx_softwarewatchdogtimer` |
| `swt2` | `0x40470000` | `s32k3xx_softwarewatchdogtimer` |
| `swt3` | `0x40070000` | `s32k3xx_softwarewatchdogtimer` |
| `stm0` | `0x40274000` | `s32k3xx_systemtimermodule` |
| `stm1` | `0x40474000` | `s32k3xx_systemtimermodule` |
| `stm2` | `0x40478000` | `s32k3xx_systemtimermodule` |
| `stm3` | `0x4047c000` | `s32k3xx_systemtimermodule` |
| `pit0` | `0x400b0000` | `s32k3xx_periodicinterrupttimer` |
| `pit1` | `0x400b4000` | `s32k3xx_periodicinterrupttimer` |
| `pit2` | `0x402fc000` | `s32k3xx_periodicinterrupttimer` |
| `pit3` | `0x40300000` | `s32k3xx_periodicinterrupttimer` |
| `rtc` | `0x40288000` | `s32k3xx_realtimeclock` |
| `lpi2c0` | `0x40350000` | `s32k3xx_lowpowerinterintegratedcircuit` |
| `lpi2c1` | `0x40354000` | `s32k3xx_lowpowerinterintegratedcircuit` |
| `lpspi0` | `0x40358000` | `imxrt_lpspi` |
| `lpspi1` | `0x4035c000` | `imxrt_lpspi` |
| `lpspi2` | `0x40360000` | `imxrt_lpspi` |
| `lpspi3` | `0x40364000` | `imxrt_lpspi` |
| `lpspi4` | `0x404bc000` | `imxrt_lpspi` |
| `lpspi5` | `0x404c0000` | `imxrt_lpspi` |
| `flex_io` | `0x40324000` | `s32k3xx_flexio` |
| `lpuart0` | `0x40328000` | `nxp_lpuart` |
| `lpuart1` | `0x4032c000` | `nxp_lpuart` |
| `lpuart2` | `0x40330000` | `nxp_lpuart` |
| `lpuart3` | `0x40334000` | `nxp_lpuart` |
| `lpuart4` | `0x40338000` | `nxp_lpuart` |
| `lpuart5` | `0x4033c000` | `nxp_lpuart` |
| `lpuart6` | `0x40340000` | `nxp_lpuart` |
| `lpuart7` | `0x40344000` | `nxp_lpuart` |
| `lpuart8` | `0x4048c000` | `nxp_lpuart` |
| `lpuart9` | `0x40490000` | `nxp_lpuart` |
| `lpuart10` | `0x40494000` | `nxp_lpuart` |
| `lpuart11` | `0x40498000` | `nxp_lpuart` |
| `lpuart12` | `0x4049c000` | `nxp_lpuart` |
| `lpuart13` | `0x404a0000` | `nxp_lpuart` |
| `lpuart14` | `0x404a4000` | `nxp_lpuart` |
| `lpuart15` | `0x404a8000` | `nxp_lpuart` |
| `mc_cgm_mux_0_css` | `0x402d8304` | `pythonperipheral` |
| `mc_cgm_mux_1_css` | `0x402d8344` | `pythonperipheral` |
| `mc_cgm_mux_2_css` | `0x402d8384` | `pythonperipheral` |
| `mc_cgm_mux_3_css` | `0x402d83c4` | `pythonperipheral` |
| `mc_cgm_mux_4_css` | `0x402d8404` | `pythonperipheral` |
| `mc_cgm_mux_5_css` | `0x402d8444` | `pythonperipheral` |
| `mc_cgm_mux_6_css` | `0x402d8484` | `pythonperipheral` |
| `mc_cgm_mux_7_css` | `0x402d84c4` | `pythonperipheral` |
| `mc_cgm_mux_8_css` | `0x402d8504` | `pythonperipheral` |
| `mc_cgm_mux_9_css` | `0x402d8544` | `pythonperipheral` |
| `mc_cgm_mux_10_css` | `0x402d8584` | `pythonperipheral` |
| `mc_cgm_mux_11_css` | `0x402d85c4` | `pythonperipheral` |
| `mc_cgm_mux_12_css` | `0x402d8604` | `pythonperipheral` |
| `mc_cgm_mux_13_css` | `0x402d8644` | `pythonperipheral` |
| `mc_cgm_mux_14_css` | `0x402d8684` | `pythonperipheral` |
| `mc_cgm_mux_15_css` | `0x402d86c4` | `pythonperipheral` |
| `mc_cgm_mux_16_css` | `0x402d8704` | `pythonperipheral` |
| `mc_cgm_mux_17_css` | `0x402d8744` | `pythonperipheral` |
| `mc_cgm_mux_18_css` | `0x402d8784` | `pythonperipheral` |
| `mc_cgm_mux_19_css` | `0x402d87c4` | `pythonperipheral` |
| `gmac0` | `0x40484000` | `s32k3xx_gmac` |
| `gmac1` | `0x40488000` | `s32k3xx_gmac` |
| `dma_channels0_11` | `0x40210000` | `nxp_edma_channels` |
| `dma_channels12_32` | `0x40410000` | `nxp_edma_channels` |
| `dma_mux0` | `0x40280000` | `s32k3xx_dmamux` |
| `dma_mux1` | `0x40284000` | `s32k3xx_dmamux` |
| `xrdc` | `0x40278000` | `nxp_xrdc` |

**Total Peripherals:** 77