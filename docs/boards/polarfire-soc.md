# Validation Report: polarfire-soc

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
| `pdma` | `0x03000000` | `mpfs_pdma` |
| `mmuart0` | `0x20000000` | `ns16550` |
| `mmuart1` | `0x20100000` | `ns16550` |
| `mmuart2` | `0x20102000` | `ns16550` |
| `mmuart3` | `0x20104000` | `ns16550` |
| `mmuart4` | `0x20106000` | `ns16550` |
| `mmc` | `0x20008000` | `mpfs_sdcontroller` |
| `spi0` | `0x20108000` | `mpfs_spi` |
| `spi1` | `0x20109000` | `mpfs_spi` |
| `i2c0` | `0x2010a000` | `mpfs_i2c` |
| `i2c1` | `0x2010b000` | `mpfs_i2c` |
| `can0` | `0x2010c000` | `mpfs_can` |
| `can1` | `0x2010d000` | `mpfs_can` |
| `gpio0` | `0x20120000` | `mpfs_gpio` |
| `gpio1` | `0x20121000` | `mpfs_gpio` |
| `gpio2` | `0x20122000` | `mpfs_gpio` |
| `wdog0` | `0x20001000` | `mpfs_watchdog` |
| `wdog1` | `0x20101000` | `mpfs_watchdog` |
| `wdog2` | `0x20103000` | `mpfs_watchdog` |
| `wdog3` | `0x20105000` | `mpfs_watchdog` |
| `wdog4` | `0x20107000` | `mpfs_watchdog` |
| `rtc` | `0x20124000` | `mpfs_rtc` |
| `mstimer` | `0x20125000` | `mpfs_timer` |
| `envmCfg` | `0x20200000` | `mpfs_envm` |
| `usb` | `0x20201000` | `mpfs_usb` |
| `pcie0` | `0x53004000` | `mpfs_pcie` |
| `pcieRC0` | `0x00000000` | `pcierootcomplex` |
| `pcieRC1` | `0x00000000` | `pcierootcomplex` |
| `pcieMem` | `0x00000001` | `pciememory` |
| `mailbox` | `0x37020800` | `arraymemory` |
| `ioscb` | `0x37080000` | `pythonperipheral` |
| `DDR_CTRLR` | `0x3e001000` | `mpfs_ddrmock` |
| `DDR_PHY` | `0x20007000` | `mpfs_ddrmock` |
| `SCB_DDR_PLL` | `0x3e010000` | `mpfs_ddrmock` |
| `DDRCFG` | `0x20080000` | `mpfs_ddrmock` |
| `CacheConfig_WayEnable` | `0x02010008` | `pythonperipheral` |
| `TopSystemRegisters` | `0x20002000` | `mpfs_sysreg` |

**Total Peripherals:** 37