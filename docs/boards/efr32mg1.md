# Validation Report: efr32mg1

**Architecture:** ARM Cortex-M4F.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
Simulation completed successfully with no warnings.
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `i2c0` | `0x4000c000` | `efr32_i2ccontroller` |
| `usart0` | `0x40010000` | `efr32_usart` |
| `usart1` | `0x40010400` | `efr32_usart` |
| `leUart0` | `0x4004a000` | `leuart` |
| `gpioPort` | `0x4000a000` | `efr32_gpioport` |
| `bitband_peripherals` | `0x42000000` | `bitbanding` |
| `bitclear` | `0x44000000` | `bitaccess` |
| `bitset` | `0x46000000` | `bitaccess` |
| `bitband_sram` | `0x22000000` | `bitbanding` |
| `timer0` | `0x40018000` | `efr32_timer` |
| `timer1` | `0x40018400` | `efr32_timer` |
| `cmu` | `0x400e4000` | `efr32_cmu` |
| `ldma` | `0x400e2000` | `efr32mg12_ldma` |

**Total Peripherals:** 13