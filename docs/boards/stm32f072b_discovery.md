# Validation Report: stm32f072b_discovery

**Architecture:** ARMv6-M.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
Simulation completed successfully with no warnings.
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `led` | `0x00000006` | `led` |
| `usart1` | `0x40013800` | `stm32f7_usart` |
| `usart2` | `0x40004400` | `stm32f7_usart` |
| `usart3` | `0x40004800` | `stm32f7_usart` |
| `usart4` | `0x40004c00` | `stm32f7_usart` |
| `usart5` | `0x40005000` | `stm32f7_usart` |
| `usart6` | `0x40011400` | `stm32f7_usart` |
| `usart7` | `0x40011800` | `stm32f7_usart` |
| `gpioPortA` | `0x48000000` | `stm32_gpioport` |
| `gpioPortB` | `0x48000400` | `stm32_gpioport` |
| `gpioPortC` | `0x48000800` | `stm32_gpioport` |
| `gpioPortD` | `0x48000c00` | `stm32_gpioport` |
| `gpioPortE` | `0x48001000` | `stm32_gpioport` |
| `gpioPortF` | `0x48001400` | `stm32_gpioport` |
| `i2c1` | `0x40005400` | `stm32f7_i2c` |
| `i2c2` | `0x40005800` | `stm32f7_i2c` |
| `spi1` | `0x40013000` | `stm32spi` |
| `spi2` | `0x40003800` | `stm32spi` |
| `timer1` | `0x40012c00` | `stm32_timer` |
| `timer2` | `0x40000000` | `stm32_timer` |
| `timer3` | `0x40000400` | `stm32_timer` |
| `timer6` | `0x40001000` | `stm32_timer` |
| `timer7` | `0x40001400` | `stm32_timer` |
| `timer14` | `0x40002000` | `stm32_timer` |
| `timer15` | `0x40014000` | `stm32_timer` |
| `timer16` | `0x40014400` | `stm32_timer` |
| `timer17` | `0x40014800` | `stm32_timer` |
| `can` | `0x40006400` | `stmcan` |
| `rtc` | `0x40002800` | `stm32f4_rtc` |
| `rcc` | `0x40021000` | `pythonperipheral` |
| `DMA` | `0x40020000` | `pythonperipheral` |
| `adc` | `0x40012400` | `stm32f0_adc` |
| `crc` | `0x40023000` | `stm32_crc` |

**Total Peripherals:** 33