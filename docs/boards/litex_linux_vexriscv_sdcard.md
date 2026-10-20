# Validation Report: litex_linux_vexriscv_sdcard

**Architecture:** RISC-V RV32IMA.

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
| `uart` | `0xf0001000` | `litex_uart` |
| `spi` | `0xf0007800` | `stub` |
| `i2c` | `0xf0008000` | `stub` |
| `si7021` | `0x00000005` | `si70xx` |
| `gpio_out` | `0xf0004800` | `litex_gpio` |
| `gpio_in` | `0xf0007000` | `litex_gpio` |
| `button` | `0x00000000` | `button` |
| `soc_controller` | `0xf0000000` | `litex_soc_controller` |
| `mmcm` | `0xf0009800` | `stub` |

**Total Peripherals:** 9