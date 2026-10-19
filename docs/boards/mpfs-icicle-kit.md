# Validation Report: mpfs-icicle-kit

**Architecture:** RISC-V RV64GC.

## 1. Dynamic Simulation Validation
**Status:** ✅ Structural Check Passed (structural-ok)

```json
{
  "system_manifest": true,
  "chip_descriptor": true,
  "flash_base_present": true,
  "ram_base_present": true
}
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `led0` | `0x00000010` | `led` |
| `led1` | `0x00000011` | `led` |
| `led2` | `0x00000012` | `led` |
| `led3` | `0x00000013` | `led` |
| `button1` | `0x0000001e` | `button` |
| `button2` | `0x0000001f` | `button` |
| `button3` | `0x0000001c` | `button` |
| `pac1934` | `0x00010000` | `pac1934` |
| `qspi` | `0x21000000` | `stub` |
| `sysServices` | `0x37020000` | `stub` |
| `syscontroller_qspi` | `0x37020100` | `stub` |
| `pdma` | `0x03000000` | `mpfs_pdma` |
| `mmuart0` | `0x20000000` | `ns16550` |
| `mmuart1` | `0x20100000` | `ns16550` |
| `mmuart2` | `0x20102000` | `ns16550` |
| `mmuart3` | `0x20104000` | `ns16550` |
| `mmuart4` | `0x20106000` | `ns16550` |
| `mmc` | `0x20008000` | `mpfs_sdcontroller` |
| `spi0` | `0x20108000` | `stub` |
| `spi1` | `0x20109000` | `stub` |
| `i2c0` | `0x2010a000` | `stub` |
| `i2c1` | `0x2010b000` | `stub` |
| `can0` | `0x2010c000` | `stub` |
| `can1` | `0x2010d000` | `stub` |
| `gpio0` | `0x20120000` | `mpfs_gpio` |
| `gpio1` | `0x20121000` | `mpfs_gpio` |
| `gpio2` | `0x20122000` | `mpfs_gpio` |
| `wdog0` | `0x20001000` | `stub` |
| `wdog1` | `0x20101000` | `stub` |
| `wdog2` | `0x20103000` | `stub` |
| `wdog3` | `0x20105000` | `stub` |
| `wdog4` | `0x20107000` | `stub` |
| `rtc` | `0x20124000` | `stub` |
| `mstimer` | `0x20125000` | `mpfs_timer` |
| `envmCfg` | `0x20200000` | `stub` |
| `usb` | `0x20201000` | `stub` |
| `pcie0` | `0x53004000` | `stub` |
| `pcieRC0` | `0x00000000` | `stub` |
| `pcieRC1` | `0x00000000` | `stub` |
| `pcieMem` | `0x00000001` | `stub` |
| `mailbox` | `0x37020800` | `arraymemory` |
| `ioscb` | `0x37080000` | `pythonperipheral` |
| `DDR_CTRLR` | `0x3e001000` | `stub` |
| `DDR_PHY` | `0x20007000` | `stub` |
| `SCB_DDR_PLL` | `0x3e010000` | `stub` |
| `DDRCFG` | `0x20080000` | `stub` |
| `CacheConfig_WayEnable` | `0x02010008` | `pythonperipheral` |
| `TopSystemRegisters` | `0x20002000` | `stub` |

**Total Peripherals:** 48