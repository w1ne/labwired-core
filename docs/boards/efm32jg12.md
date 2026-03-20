# Validation Report: efm32jg12

**Architecture:** ARM Cortex-M3.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
Simulation completed successfully with no warnings.
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `i2c0` | `0x4000c000` | `efm32ggi2ccontroller` |
| `i2c1` | `0x4000c400` | `efm32ggi2ccontroller` |
| `timer0` | `0x40018000` | `efm32timer` |
| `timer1` | `0x40018400` | `efm32timer` |
| `usart0` | `0x40010000` | `efm32_uart` |
| `usart1` | `0x40010400` | `efm32_uart` |
| `usart2` | `0x40010800` | `efm32_uart` |
| `usart3` | `0x40010c00` | `efm32_uart` |
| `leUart0` | `0x4004a000` | `leuart` |
| `gpioPort` | `0x4000a000` | `efmgpioport` |
| `bitband` | `0x42000000` | `bitbanding` |
| `cmuStatusOscillator` | `0x400c802c` | `pythonperipheral` |
| `vcmpStatus` | `0x40000008` | `pythonperipheral` |

**Total Peripherals:** 13