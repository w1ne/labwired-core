# Validation Report: picosoc

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
| `uart` | `0x02000004` | `picosoc_simpleuart` |

**Total Peripherals:** 1