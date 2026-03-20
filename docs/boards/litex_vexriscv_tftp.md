# Validation Report: litex_vexriscv_tftp

**Architecture:** RISC-V RV32IMA.

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
| `ctrl` | `0x82000000` | `litex_soc_controller` |
| `uart` | `0x82002000` | `litex_uart` |
| `timer0` | `0x82002800` | `litex_timer` |
| `ethphy` | `0x00000000` | `ethernetphysicallayer` |

**Total Peripherals:** 4