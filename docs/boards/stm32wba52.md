# Validation Report: stm32wba52

**Architecture:** ARMv8-M.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
Simulation completed successfully with no warnings.
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `pwr` | `0x46020800` | `stm32wba_pwr` |
| `usart1` | `0x40013800` | `stm32f7_usart` |
| `usart2` | `0x40004400` | `stm32f7_usart` |
| `lpuart1` | `0x46002400` | `stm32f7_usart` |
| `spi1` | `0x40013000` | `stm32wba_spi` |
| `spi3` | `0x46002000` | `stm32wba_spi` |
| `adc4` | `0x46021000` | `stm32wba_adc` |
| `gpioPortA` | `0x42020000` | `stm32_gpioport` |
| `gpioPortB` | `0x42020400` | `stm32_gpioport` |
| `gpioPortC` | `0x42020800` | `stm32_gpioport` |
| `gpioPortH` | `0x42021c00` | `stm32_gpioport` |
| `i2c1` | `0x40005400` | `stm32f7_i2c` |
| `i2c3` | `0x46002800` | `stm32f7_i2c` |
| `timer1` | `0x40012c00` | `stm32_timer` |
| `timer2` | `0x40000000` | `stm32_timer` |
| `timer3` | `0x40000400` | `stm32_timer` |
| `timer16` | `0x40014400` | `stm32_timer` |
| `timer17` | `0x40014800` | `stm32_timer` |
| `iwdg` | `0x40003000` | `stm32_independentwatchdog` |
| `lptim1` | `0x46004400` | `stm32l0_lptimer` |
| `lptim2` | `0x40009400` | `stm32l0_lptimer` |
| `rtc` | `0x46007800` | `stm32f4_rtc` |
| `flash_ctrl` | `0x40022000` | `stm32wba_flashcontroller` |
| `rcc` | `0x46020c00` | `stm32wba_rcc` |

**Total Peripherals:** 24