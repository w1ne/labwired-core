# Validation Report: verilated_ibex

**Architecture:** Unknown

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
| `rom` | `0x00000000` | `arraymemory` |
| `sram` | `0x01000000` | `arraymemory` |
| `main_ram` | `0x40000000` | `arraymemory` |
| `stack` | `0xf0000000` | `arraymemory` |
| `cpu` | `0x0000000c` | `cosimulatedriscv32` |
| `ctrl` | `0x82000000` | `litex_soc_controller` |
| `uart` | `0x82002000` | `litex_uart` |
| `timer0` | `0x82002800` | `litex_timer_csr32` |

**Total Peripherals:** 8