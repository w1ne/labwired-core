# Validation Report: opentitan-earlgrey

**Architecture:** RISC-V RV32IMC.

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
| `rom_ctrl` | `0x411e0000` | `opentitan_romcontroller` |
| `sram_ctrl` | `0x411c0000` | `opentitan_sramcontroller` |
| `flash_ctrl` | `0x41000000` | `opentitan_flashcontroller` |
| `uart0` | `0x40000000` | `opentitan_uart` |
| `uart1` | `0x40010000` | `opentitan_uart` |
| `uart2` | `0x40020000` | `opentitan_uart` |
| `uart3` | `0x40030000` | `opentitan_uart` |
| `i2c0` | `0x40080000` | `stub` |
| `i2c1` | `0x40090000` | `stub` |
| `i2c2` | `0x400a0000` | `stub` |
| `spi_device` | `0x40050000` | `stub` |
| `gpio` | `0x40040000` | `opentitan_gpio` |
| `aes` | `0x41100000` | `stub` |
| `keymgr` | `0x41140000` | `stub` |
| `csrng` | `0x41150000` | `stub` |
| `hmac` | `0x41110000` | `stub` |
| `kmac` | `0x41120000` | `stub` |
| `rv_timer` | `0x40100000` | `opentitan_timer` |
| `timer_aon` | `0x40470000` | `opentitan_aontimer` |
| `pwrmgr_aon` | `0x40400000` | `stub` |
| `rstmgr_aon` | `0x40410000` | `stub` |
| `otp_ctrl` | `0x40130000` | `stub` |
| `lc_ctrl` | `0x40140000` | `stub` |
| `swteststatus` | `0x411f0080` | `stub` |
| `entropy_src` | `0x41160000` | `stub` |
| `edn0` | `0x41170000` | `stub` |
| `edn1` | `0x41180000` | `stub` |
| `alert_handler` | `0x40150000` | `stub` |
| `otbn` | `0x41130000` | `stub` |
| `sysrst_ctrl` | `0x40430000` | `stub` |
| `clock_manager` | `0x40420000` | `stub` |
| `RV_CORE_IBEX_RND_DATA` | `0x411f0058` | `pythonperipheral` |

**Total Peripherals:** 32