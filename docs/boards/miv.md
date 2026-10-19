# Validation Report: miv

**Architecture:** RISC-V.

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
| `uart` | `0x70001000` | `miv_coreuart` |
| `gpioInputs` | `0x70002000` | `miv_coregpio` |
| `gpioOutputs` | `0x70005000` | `miv_coregpio` |
| `timer0` | `0x70003000` | `miv_coretimer` |
| `timer1` | `0x70004000` | `miv_coretimer` |

**Total Peripherals:** 5