# Validation Report: focaltech_ft9001

**Architecture:** RISC / Proprietary.

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
| `cpm` | `0x40004000` | `focaltechft9001_cpm` |
| `rst` | `0x40002000` | `focaltechft9001_reset` |
| `trng` | `0x4003b000` | `focaltechft9001_trng` |
| `usart2` | `0x40014000` | `ft9001_usart` |
| `dwt` | `0xe0001000` | `dwt` |

**Total Peripherals:** 5