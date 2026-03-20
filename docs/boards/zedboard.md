# Validation Report: zedboard

**Architecture:** Dual ARM Cortex-A9.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
Simulation completed successfully with no warnings.
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `usbEhci2` | `0xe0003000` | `ehcihostcontroller` |
| `usbEhci` | `0xe0002000` | `ehcihostcontroller` |
| `pl310` | `0xf8f02000` | `pl310` |
| `gem0` | `0xe000b000` | `cadencegem` |
| `gem1` | `0xe000c000` | `cadencegem` |
| `uart0` | `0xe0000000` | `cadence_uart` |
| `uart1` | `0xe0001000` | `cadence_uart` |
| `i2c1` | `0xe0005000` | `cadence_i2c` |
| `i2c0` | `0xe0004000` | `cadence_i2c` |
| `spi0` | `0xe0006000` | `cadence_spi` |
| `spi1` | `0xe0007000` | `cadence_spi` |
| `sdhci0` | `0xe0100000` | `sdhci` |
| `sdhci1` | `0xe0101000` | `sdhci` |
| `ttc0` | `0xf8001000` | `cadence_ttc` |
| `ttc1` | `0xf8002000` | `cadence_ttc` |
| `watchdog0` | `0xf8005000` | `cadence_wdt` |
| `globalTimer` | `0xf8f00200` | `arm_globaltimer` |
| `scu` | `0xf8f00000` | `armsnoopcontrolunit` |
| `qspi` | `0xe000d000` | `xilinxqspi` |
| `gpio` | `0xe000a000` | `xilinxgpiops` |
| `xadc` | `0xf8007100` | `xilinx_xadc` |
| `slcr` | `0xf8000000` | `zynq7000_systemlevelcontrolregisters` |
| `nand` | `0xe000e000` | `pythonperipheral` |
| `dma_pl330` | `0xf8003000` | `pl330_dma` |
| `phy` | `0x00000000` | `ethernetphysicallayer` |
| `led0` | `0x0000003d` | `led` |
| `led1` | `0x0000003e` | `led` |
| `led2` | `0x0000003f` | `led` |
| `led3` | `0x00000040` | `led` |
| `led4` | `0x00000041` | `led` |
| `led5` | `0x00000042` | `led` |
| `led6` | `0x00000043` | `led` |
| `led7` | `0x00000044` | `led` |

**Total Peripherals:** 33