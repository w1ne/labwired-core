# Validation Report: atsamd21j17d-aft

**Architecture:** ARM Cortex-M0+. General purpose low power microcontroller.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
Simulation completed successfully with no warnings.
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `dwt` | `0x0000e000` | `dwt` |
| `rtc` | `0x00004000` | `samd21_rtc` |
| `gpio_a` | `0x00004100` | `samd21_gpio` |
| `gpio_b` | `0x00004100` | `samd21_gpio` |
| `usart0` | `0x00004200` | `samd5_uart` |
| `tc4` | `0x00004200` | `samd21_timer` |
| `tc6` | `0x00004200` | `samd21_timer` |
| `gclk` | `0x40000c00` | `arraymemory` |
| `usart1` | `0x00004200` | `samd5_uart` |
| `usart2` | `0x00004200` | `samd5_uart` |

**Total Peripherals:** 10