# Validation Report: murax_vexriscv_verilated_uart

**Architecture:** RISC-V RV32I.

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
| `uart` | `0xf0010000` | `cosimulateduart` |
| `gpioA` | `0xf0000000` | `murax_gpio` |
| `timer` | `0xf0020000` | `murax_timer` |

**Total Peripherals:** 3