# Validation Report: stm32f7_discovery-bb

**Architecture:** ARMv7E-M.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
Simulation completed successfully with no warnings.
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `phy` | `0x00000000` | `ethernetphysicallayer` |
| `phy1` | `0x00000001` | `ethernetphysicallayer` |
| `touchscreen` | `0x00000038` | `ft5336` |
| `usart1` | `0x40011000` | `stm32f7_usart` |
| `usart2` | `0x40004400` | `stm32f7_usart` |
| `usart3` | `0x40004800` | `stm32f7_usart` |
| `usart6` | `0x40011400` | `stm32f7_usart` |
| `can1` | `0x40006400` | `stmcan` |
| `gpioPortA` | `0x40020000` | `stm32_gpioport` |
| `gpioPortB` | `0x40020400` | `stm32_gpioport` |
| `gpioPortC` | `0x40020800` | `stm32_gpioport` |
| `gpioPortD` | `0x40020c00` | `stm32_gpioport` |
| `gpioPortE` | `0x40021000` | `stm32_gpioport` |
| `gpioPortF` | `0x40021400` | `stm32_gpioport` |
| `gpioPortG` | `0x40021800` | `stm32_gpioport` |
| `gpioPortH` | `0x40021c00` | `stm32_gpioport` |
| `gpioPortI` | `0x40022000` | `stm32_gpioport` |
| `gpioPortJ` | `0x40022400` | `stm32_gpioport` |
| `gpioPortK` | `0x40022800` | `stm32_gpioport` |
| `ethernet` | `0x40028000` | `synopsysethernetmac` |
| `spi1` | `0x40013000` | `stm32spi` |
| `spi2` | `0x40003800` | `stm32spi` |
| `spi3` | `0x40003c00` | `stm32spi` |
| `dma1` | `0x40026000` | `stm32dma` |
| `dma2` | `0x40026400` | `stm32dma` |
| `ltdc` | `0x40016800` | `stm32ltdc` |
| `dma2d` | `0x4002b000` | `stm32dma2d` |
| `i2c1` | `0x40005400` | `stm32f7_i2c` |
| `i2c2` | `0x40005800` | `stm32f7_i2c` |
| `i2c3` | `0x40005c00` | `stm32f7_i2c` |
| `i2c4` | `0x40006000` | `stm32f7_i2c` |
| `syscfg` | `0x40013800` | `stm32_syscfg` |
| `lptim1Isr` | `0x40002400` | `pythonperipheral` |
| `rtc` | `0x40002800` | `stm32f4_rtc` |
| `rcc` | `0x40023800` | `stm32f4_rcc` |
| `rng` | `0x50060800` | `stm32f4_rng` |
| `pwrCr1` | `0x40007000` | `pythonperipheral` |
| `pwrCsr1` | `0x40007004` | `pythonperipheral` |
| `sdmmc` | `0x40012c00` | `stm32fsdmmc` |
| `timer1` | `0x40010000` | `stm32_timer` |
| `timer2` | `0x40000000` | `stm32_timer` |
| `timer3` | `0x40000400` | `stm32_timer` |
| `timer4` | `0x40000800` | `stm32_timer` |
| `timer5` | `0x40000c00` | `stm32_timer` |
| `timer6` | `0x40001000` | `stm32_timer` |
| `timer7` | `0x40001400` | `stm32_timer` |
| `timer8` | `0x40010400` | `stm32_timer` |
| `timer9` | `0x40014000` | `stm32_timer` |
| `timer10` | `0x40014400` | `stm32_timer` |
| `timer11` | `0x40014800` | `stm32_timer` |
| `timer12` | `0x40001800` | `stm32_timer` |
| `timer13` | `0x40001c00` | `stm32_timer` |
| `timer14` | `0x40002000` | `stm32_timer` |

**Total Peripherals:** 53