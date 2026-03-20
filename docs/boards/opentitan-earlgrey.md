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
| `i2c0` | `0x40080000` | `opentitan_i2c` |
| `i2c1` | `0x40090000` | `opentitan_i2c` |
| `i2c2` | `0x400a0000` | `opentitan_i2c` |
| `spi_device` | `0x40050000` | `opentitan_spidevice` |
| `gpio` | `0x40040000` | `opentitan_gpio` |
| `aes` | `0x41100000` | `opentitan_aes` |
| `keymgr` | `0x41140000` | `opentitan_keymanager` |
| `csrng` | `0x41150000` | `opentitan_csrng` |
| `hmac` | `0x41110000` | `opentitan_hmac` |
| `kmac` | `0x41120000` | `opentitan_kmac` |
| `rv_timer` | `0x40100000` | `opentitan_timer` |
| `timer_aon` | `0x40470000` | `opentitan_aontimer` |
| `pwrmgr_aon` | `0x40400000` | `opentitan_powermanager` |
| `rstmgr_aon` | `0x40410000` | `opentitan_resetmanager` |
| `otp_ctrl` | `0x40130000` | `opentitan_onetimeprogrammablememorycontroller` |
| `lc_ctrl` | `0x40140000` | `opentitan_lifecyclecontroller` |
| `swteststatus` | `0x411f0080` | `opentitan_verilatorswteststatus` |
| `entropy_src` | `0x41160000` | `opentitan_entropysource` |
| `edn0` | `0x41170000` | `opentitan_entropydistributionnetwork` |
| `edn1` | `0x41180000` | `opentitan_entropydistributionnetwork` |
| `alert_handler` | `0x40150000` | `opentitan_alerthandler` |
| `otbn` | `0x41130000` | `opentitan_bignumberaccelerator` |
| `sysrst_ctrl` | `0x40430000` | `opentitan_systemresetcontrol` |
| `clock_manager` | `0x40420000` | `opentitan_clockmanager` |
| `RV_CORE_IBEX_RND_DATA` | `0x411f0058` | `pythonperipheral` |

**Total Peripherals:** 32