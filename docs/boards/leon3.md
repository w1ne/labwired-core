# Validation Report: leon3

**Architecture:** SPARC.

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
| `ahbInfo` | `0xfffff000` | `gaislerahbplugandplayinfo` |
| `apbController` | `0x800ff000` | `gaislerapbcontroller` |
| `esaMemController` | `0x80000000` | `esamemorycontroller` |
| `uart` | `0x80000100` | `gaislerapbuart` |
| `timer` | `0x80000300` | `gaisler_gptimer` |
| `eth` | `0x80000b00` | `gaislereth` |
| `gpio` | `0x80000800` | `gaisler_gpio` |
| `phy` | `0x00000000` | `ethernetphysicallayer` |
| `led` | `0x00000000` | `led` |

**Total Peripherals:** 9