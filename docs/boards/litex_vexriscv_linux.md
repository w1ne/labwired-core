# Validation Report: litex_vexriscv_linux

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
| `timer0` | `0xf0001800` | `litex_timer` |
| `uart` | `0xf0001000` | `litex_uart` |
| `cpu_timer` | `0xf0000800` | `litex_cputimer` |
| `spi` | `0xf0008000` | `litex_spi` |

**Total Peripherals:** 4