# Validation Report: litex_minerva

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
| `boot_flash` | `0x20228000` | `arraymemory` |
| `uart` | `0xe0001800` | `litex_uart` |
| `timer0` | `0xe0002800` | `litex_timer` |

**Total Peripherals:** 3