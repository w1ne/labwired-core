# Validation Report: litex_vexriscv_verilated_liteuart

**Architecture:** RISC-V RV32IMAC.

## 1. Dynamic Simulation Validation
**Status:** ❌ Failed (missing-memory-map-or-manifest)

```json
{
  "system_manifest": true,
  "chip_descriptor": true,
  "flash_base_present": false,
  "ram_base_present": true
}
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `uart` | `0xe0001000` | `cosimulateduart` |

**Total Peripherals:** 1