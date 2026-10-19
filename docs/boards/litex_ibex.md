# Validation Report: litex_ibex

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
| `ctrl` | `0x82000000` | `litex_soc_controller` |
| `uart` | `0x82002000` | `litex_uart` |
| `timer0` | `0x82002800` | `litex_timer_csr32` |

**Total Peripherals:** 3