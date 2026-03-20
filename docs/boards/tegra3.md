# Validation Report: tegra3

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
| `uart1` | `0x70006040` | `ns16550` |
| `uart2` | `0x70006200` | `ns16550` |
| `uart3` | `0x70006300` | `ns16550` |
| `uart4` | `0x70003400` | `ns16550` |
| `dmaHost1xA` | `0x50010000` | `tegradmahost1x` |
| `dmaHost1xB` | `0x50004000` | `tegradmahost1x` |
| `privateTimer` | `0x50040600` | `arm_privatetimer` |
| `tmr1` | `0x60005000` | `tegratimer` |
| `tmr2` | `0x60005008` | `tegratimer` |
| `tmrUs` | `0x60005010` | `tegrausectimer` |
| `tmr3` | `0x60005050` | `tegratimer` |
| `tmr4` | `0x60005058` | `tegratimer` |
| `pl310` | `0x50043000` | `pl310` |
| `usbEhci1` | `0x7d000000` | `ehcihostcontroller` |
| `usbEhci2` | `0x7d004000` | `ehcihostcontroller` |
| `usbEhci3` | `0x7d008000` | `ehcihostcontroller` |
| `apbDma` | `0x6000a000` | `tegradma` |
| `i2c1` | `0x7000c000` | `tegrai2ccontroller` |
| `i2c2` | `0x7000c400` | `tegrai2ccontroller` |
| `i2c3` | `0x7000c500` | `tegrai2ccontroller` |
| `i2c4` | `0x7000c700` | `tegrai2ccontroller` |
| `i2c5` | `0x7000d000` | `tegrai2ccontroller` |
| `fb0` | `0x54200000` | `tegradisplay` |
| `fb1` | `0x54240000` | `tegradisplay` |
| `syncPts` | `0x50000000` | `tegrasyncpts` |
| `nvPaPmcBase` | `0x7000e400` | `pythonperipheral` |
| `pgUpTag0` | `0x60000000` | `pythonperipheral` |
| `memoryControllerMemsize` | `0x7000f410` | `pythonperipheral` |
| `nandHack1` | `0x70008000` | `pythonperipheral` |
| `nandHackNandStatus` | `0x70008004` | `pythonperipheral` |
| `nandHackIsr` | `0x70008008` | `pythonperipheral` |
| `oscFreqDetStatus` | `0x6000605c` | `pythonperipheral` |
| `pllC` | `0x60006080` | `pythonperipheral` |
| `pllM` | `0x60006090` | `pythonperipheral` |
| `pllP` | `0x600060a0` | `pythonperipheral` |
| `superclock1` | `0x60006368` | `pythonperipheral` |
| `superclock2` | `0x60006370` | `pythonperipheral` |
| `sdmmc3` | `0x600061bc` | `pythonperipheral` |
| `clkSrc` | `0x60006104` | `pythonperipheral` |
| `clkSrcA` | `0x60006124` | `pythonperipheral` |
| `clkSrc_` | `0x60006128` | `pythonperipheral` |
| `pllSomething2` | `0x600060d0` | `pythonperipheral` |
| `pllSomething3` | `0x600060dc` | `pythonperipheral` |
| `pllSomething` | `0x60006004` | `pythonperipheral` |
| `test111` | `0x7000f204` | `pythonperipheral` |
| `apbMiscGpHidrev0` | `0x70000804` | `pythonperipheral` |
| `clockHack` | `0x60006020` | `pythonperipheral` |
| `clkRstControllerSclkBurstpolicy0` | `0x60006028` | `pythonperipheral` |
| `kfuseHack` | `0x7000f800` | `pythonperipheral` |
| `kfuseHack2` | `0x7000f9fc` | `pythonperipheral` |
| `kfuseSkuInfo` | `0x7000f910` | `pythonperipheral` |
| `kfuseTestProgRevision` | `0x7000f928` | `pythonperipheral` |
| `fuseSpeedoCalib` | `0x7000f914` | `pythonperipheral` |
| `pwrGateStatus` | `0x7000e438` | `pythonperipheral` |
| `miscDebug` | `0x70000014` | `pythonperipheral` |
| `tegraId` | `0x70000860` | `pythonperipheral` |
| `usbHub` | `0x00000001` | `usbhub` |
| `usbMouse` | `0x00000002` | `usbmouse` |
| `usbKeyboard` | `0x00000001` | `usbkeyboard` |

**Total Peripherals:** 60