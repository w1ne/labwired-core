# Validation Report: core-v-mcu

**Architecture:** RISC-V RV32IMFC.

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
| `uart` | `0x1a102100` | `pulp_udma_uart` |
| `timer` | `0x1a10b000` | `pulp_timer` |
| `gpio` | `0x1a101000` | `pulp_apb_gpio` |
| `i2s` | `0x1a102200` | `pulp_i2s` |
| `i2c0` | `0x1a102180` | `pulp_udma_i2c` |
| `spi` | `0x1a102080` | `pulp_udma_spi` |
| `camera_controller` | `0x1a102280` | `pulp_udma_camera` |

**Total Peripherals:** 7