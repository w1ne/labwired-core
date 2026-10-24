# Validation Report: nucleo_f767zi

**Architecture:** ARM Cortex-M.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
[2m2026-10-18T14:40:06.736735Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting LabWired Simulator
[2m2026-10-18T14:40:06.740569Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../chips/onboarding/nucleo_f767zi.yaml
[2m2026-10-18T14:40:06.750199Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Loading firmware: "/home/andrii/Projects/labwired-renode/core/tests/fixtures/uart-ok-thumbv7m.elf"
[2m2026-10-18T14:40:06.750303Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x401
[2m2026-10-18T14:40:06.750356Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Firmware Loaded Successfully!
[2m2026-10-18T14:40:06.750366Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Entry Point: 0x401
[2m2026-10-18T14:40:06.759674Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting Simulation (ARM Cortex-M)...
[2m2026-10-18T14:40:06.759694Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Initial PC: 0x400, SP: 0x20020000
[2m2026-10-18T14:40:06.759701Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Running for 20000 steps...
[2m2026-10-18T14:40:06.771194Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Progress: 10000 steps, current IPS: 480321.98
[2m2026-10-18T14:40:06.798331Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Simulation loop finished.
[2m2026-10-18T14:40:06.798352Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Final PC: 0x466
[2m2026-10-18T14:40:06.798358Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Instructions: 19999
[2m2026-10-18T14:40:06.798363Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Cycles: 20002
[2m2026-10-18T14:40:06.798367Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Average IPS: 416682.67
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `GreenLED` | `0x00000000` | `led` |
| `BlueLED` | `0x00000007` | `led` |
| `RedLED` | `0x0000000e` | `led` |
| `UserButton1` | `0x0000000d` | `button` |
| `phy` | `0x00000000` | `ethernetphysicallayer` |
| `flash_controller` | `0x40023c00` | `stm32f4_flashcontroller` |
| `usart1` | `0x40011000` | `stm32f7_usart` |
| `usart2` | `0x40004400` | `stm32f7_usart` |
| `usart3` | `0x40004800` | `stm32f7_usart` |
| `uart4` | `0x40004c00` | `stm32_uart` |
| `uart5` | `0x40005000` | `stm32_uart` |
| `usart6` | `0x40011400` | `stm32f7_usart` |
| `uart7` | `0x40007c00` | `stm32_uart` |
| `uart8` | `0x40007800` | `stm32_uart` |
| `can1` | `0x40006400` | `stmcan` |
| `can2` | `0x40006800` | `stmcan` |
| `can3` | `0x40003400` | `stmcan` |
| `dwt` | `0xe0001000` | `dwt` |
| `adc1` | `0x40012000` | `stm32_adc` |
| `adc2` | `0x40012100` | `stm32_adc` |
| `adc3` | `0x40012200` | `stm32_adc` |
| `crc` | `0x40023000` | `stm32_crc` |
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
| `spi4` | `0x40013400` | `stm32spi` |
| `spi5` | `0x40015000` | `stm32spi` |
| `spi6` | `0x40015400` | `stm32spi` |
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
| `rng` | `0x50060800` | `stm32_rng` |
| `pwrCr1` | `0x40007000` | `pythonperipheral` |
| `pwrCsr1` | `0x40007004` | `pythonperipheral` |
| `sdMmcSta` | `0x40012c34` | `pythonperipheral` |
| `timer5` | `0x40000c24` | `pythonperipheral` |
| `timer6` | `0x40001000` | `stm32_timer` |

**Total Peripherals:** 58