# Validation Report: up_squared_x86_64

**Architecture:** x86_64.

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
| `uart` | `0xa0000000` | `ns16550` |
| `hpet` | `0xfed00000` | `hpet` |
| `pci` | `0xe0000cf8` | `pythonperipheral` |

**Total Peripherals:** 3