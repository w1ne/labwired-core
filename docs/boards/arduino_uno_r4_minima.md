# Validation Report: arduino_uno_r4_minima

**Architecture:** ARM Cortex-M4F. Classic Uno hardware with Cortex-M.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
Simulation completed successfully with no warnings.
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `gpt` | `0x40078000` | `renesasra_gpt` |
| `agt0` | `0x40084000` | `renesasra_agt` |
| `agt1` | `0x40084100` | `renesasra_agt` |
| `sci0` | `0x40070000` | `renesasra6m5_sci` |
| `sci1` | `0x40070020` | `renesasra6m5_sci` |
| `sci2` | `0x40070040` | `renesasra6m5_sci` |
| `sci9` | `0x40070120` | `renesasra6m5_sci` |
| `portMisc` | `0x40040d00` | `renesasra_gpiomisc` |
| `sysc_oscsf` | `0x4001e03c` | `pythonperipheral` |
| `sysc_vbtsr` | `0x4001e4b1` | `pythonperipheral` |
| `iic0` | `0x40053000` | `renesasra_iic` |
| `iic1` | `0x40053100` | `renesasra_iic` |

**Total Peripherals:** 12