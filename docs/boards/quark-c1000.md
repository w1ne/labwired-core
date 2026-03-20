# Validation Report: quark-c1000

**Architecture:** x86/ARC.

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
| `spi0` | `0xb0001000` | `quark_spi` |
| `spi1` | `0xb0001400` | `quark_spi` |
| `uartA` | `0xb0002000` | `ns16550` |
| `uartB` | `0xb0002400` | `ns16550` |
| `gpio` | `0xb0000c00` | `quark_gpiocontroller` |
| `pwm` | `0xb0000800` | `quark_pwm` |
| `scss` | `0xb0800000` | `quark_systemcontrolsubsystem` |

**Total Peripherals:** 7