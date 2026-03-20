# Validation Report: slwstk6220a

**Architecture:** ARM Cortex-M4F.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
Simulation completed successfully with no warnings.
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `led0` | `0x00000042` | `led` |
| `led1` | `0x00000043` | `led` |
| `button0` | `0x00000040` | `button` |
| `button1` | `0x00000041` | `button` |
| `si7021` | `0x00000080` | `si70xx` |
| `i2c0` | `0x4000a000` | `efm32ggi2ccontroller` |
| `i2c1` | `0x4000a400` | `efm32ggi2ccontroller` |
| `timer0` | `0x40010000` | `efm32timer` |
| `timer1` | `0x40010400` | `efm32timer` |
| `timer2` | `0x40010800` | `efm32timer` |
| `timer3` | `0x40010c00` | `efm32timer` |
| `uart0` | `0x4000e000` | `efm32_uart` |
| `uart1` | `0x4000e400` | `efm32_uart` |
| `usart1` | `0x4000c400` | `efm32_uart` |
| `usart2` | `0x4000c800` | `efm32_uart` |
| `leUart0` | `0x40084000` | `leuart` |
| `leUart1` | `0x40084400` | `leuart` |
| `gpioPort` | `0x40006000` | `efmgpioport` |
| `bitband` | `0x42000000` | `bitbanding` |

**Total Peripherals:** 19