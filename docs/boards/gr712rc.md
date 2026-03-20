# Validation Report: gr712rc

**Architecture:** SPARC V8.

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
| `ftmctrl` | `0x80000000` | `gaisler_faulttolerantmemorycontroller` |
| `uart0` | `0x80000100` | `gaislerapbuart` |
| `uart1` | `0x80100100` | `gaislerapbuart` |
| `uart2` | `0x80100200` | `gaislerapbuart` |
| `uart3` | `0x80100300` | `gaislerapbuart` |
| `uart4` | `0x80100400` | `gaislerapbuart` |
| `uart5` | `0x80100500` | `gaislerapbuart` |
| `gptimer` | `0x80000300` | `gaisler_gptimer` |
| `grtimer` | `0x80100600` | `gaisler_gptimer` |
| `gpio1` | `0x80000900` | `gaisler_gpio` |
| `gpio2` | `0x80000a00` | `gaisler_gpio` |
| `greth` | `0x80000e00` | `gaislereth` |
| `phy` | `0x00000000` | `ethernetphysicallayer` |
| `ahbInfo` | `0xfffff000` | `gaislerahbplugandplayinfo` |
| `apb1Controller` | `0x800ff000` | `gaislerapbcontroller` |
| `apb2Controller` | `0x801ff000` | `gaislerapbcontroller` |

**Total Peripherals:** 16