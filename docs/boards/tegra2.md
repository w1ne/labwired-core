# Validation Report: tegra2

**Architecture:** ARMv7-A (Cortex-A9).

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
Simulation completed successfully with no warnings.
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `uart0` | `0x70006000` | `ns16550` |
| `uart3` | `0x70006300` | `ns16550` |
| `dmaHost1xA` | `0x50010000` | `tegradmahost1x` |
| `dmaHost1xB` | `0x50004000` | `tegradmahost1x` |
| `privateTimer` | `0x50040600` | `arm_privatetimer` |
| `tmr1` | `0x60005000` | `tegratimer` |
| `tmr2` | `0x60005008` | `tegratimer` |
| `tmrUs` | `0x60005010` | `tegrausectimer` |
| `tmr3` | `0x60005050` | `tegratimer` |
| `tmr4` | `0x60005058` | `tegratimer` |
| `pl310` | `0x50043000` | `pl310` |
| `scu` | `0xf8f00000` | `armsnoopcontrolunit` |
| `usbEhci1` | `0xc5000000` | `ehcihostcontroller` |
| `usbEhci2` | `0xc5004000` | `ehcihostcontroller` |
| `usbEhci3` | `0xc5008000` | `ehcihostcontroller` |
| `apbDma` | `0x6000a000` | `tegradma` |
| `i2c1` | `0x7000c000` | `tegrai2ccontroller` |
| `i2c2` | `0x7000c400` | `tegrai2ccontroller` |
| `i2c3` | `0x7000c500` | `tegrai2ccontroller` |
| `fb1` | `0x54240000` | `tegradisplay` |
| `syncPts` | `0x50000000` | `tegrasyncpts` |
| `dvc` | `0x7000d000` | `tegradvc` |
| `pwrGateStatus` | `0x7000e438` | `pythonperipheral` |
| `memoryControllerMemsize` | `0x7000f410` | `pythonperipheral` |
| `nandHack1` | `0x70008000` | `pythonperipheral` |
| `nandHackNandStatus` | `0x70008004` | `pythonperipheral` |
| `nanHackIsr` | `0x70008008` | `pythonperipheral` |
| `oscFreqDetStatus` | `0x6000605c` | `pythonperipheral` |
| `pllC` | `0x60006080` | `pythonperipheral` |
| `pllM` | `0x60006090` | `pythonperipheral` |
| `pllP` | `0x600060a0` | `pythonperipheral` |
| `clkSrc` | `0x60006104` | `pythonperipheral` |
| `apbMiscGpHidrev0` | `0x70000804` | `pythonperipheral` |
| `clockHack` | `0x60006020` | `pythonperipheral` |
| `clkRstControllerSclkBurstPolicy0` | `0x60006028` | `pythonperipheral` |
| `kfuseHack` | `0x7000f800` | `pythonperipheral` |
| `usbHub` | `0x00000001` | `usbhub` |
| `usbMouse` | `0x00000002` | `usbmouse` |
| `usbKeyboard` | `0x00000001` | `usbkeyboard` |

**Total Peripherals:** 39