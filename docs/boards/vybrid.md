# Validation Report: vybrid

**Architecture:** ARMv7-A / ARMv7-M.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
Simulation completed successfully with no warnings.
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `uart0` | `0x40027000` | `imxuart` |
| `uart1` | `0x40028000` | `imxuart` |
| `uart2` | `0x40029000` | `imxuart` |
| `eDma0` | `0x40018000` | `vybriddma` |
| `eDma1` | `0x40098000` | `vybriddma` |
| `fb` | `0x40058000` | `vybriddcu` |
| `usbEhci0` | `0x40034000` | `ehcihostcontroller` |
| `usbEhci1` | `0x400b4000` | `ehcihostcontroller` |
| `i2c0` | `0x40066000` | `vybridi2c` |
| `i2c1` | `0x40067000` | `vybridi2c` |
| `timers` | `0x40037000` | `periodicinterrupttimer` |
| `globalTimer` | `0x40002200` | `arm_globaltimer` |
| `sema4` | `0x4001d000` | `sema4` |
| `nand` | `0x400e0000` | `fslnand` |

**Total Peripherals:** 14