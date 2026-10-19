# Validation Report: stm32f4_discovery

**Architecture:** ARMv7E-M.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
[2m2026-10-18T14:40:27.655099Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting LabWired Simulator
[2m2026-10-18T14:40:27.657671Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../chips/onboarding/stm32f4_discovery.yaml
[2m2026-10-18T14:40:27.665733Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Loading firmware: "/home/andrii/Projects/labwired-renode/core/tests/fixtures/uart-ok-thumbv7m.elf"
[2m2026-10-18T14:40:27.665836Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x401
[2m2026-10-18T14:40:27.665856Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Firmware Loaded Successfully!
[2m2026-10-18T14:40:27.665861Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Entry Point: 0x401
[2m2026-10-18T14:40:27.703741Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting Simulation (ARM Cortex-M)...
[2m2026-10-18T14:40:27.703760Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Initial PC: 0x400, SP: 0x20020000
[2m2026-10-18T14:40:27.703766Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Running for 20000 steps...
[2m2026-10-18T14:40:27.708316Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Progress: 10000 steps, current IPS: 235575.89
[2m2026-10-18T14:40:27.713059Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Simulation loop finished.
[2m2026-10-18T14:40:27.713218Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Final PC: 0x466
[2m2026-10-18T14:40:27.713250Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Instructions: 19999
[2m2026-10-18T14:40:27.713276Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Cycles: 20002
[2m2026-10-18T14:40:27.713303Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Average IPS: 421590.17
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `usart1` | `0x40011000` | `stm32_uart` |
| `usart2` | `0x40004400` | `stm32_uart` |
| `usart3` | `0x40004800` | `stm32_uart` |
| `uart4` | `0x40004c00` | `stm32_uart` |
| `uart5` | `0x40005000` | `stm32_uart` |
| `can1` | `0x40006400` | `stmcan` |
| `can2` | `0x40006800` | `stmcan` |
| `pwr` | `0x40007000` | `stm32_pwr` |
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
| `i2c1` | `0x40005400` | `stm32f1_i2c` |
| `i2c2` | `0x40005800` | `stm32f1_i2c` |
| `i2c3` | `0x40005c00` | `stm32f1_i2c` |
| `dma1` | `0x40026000` | `stm32dma` |
| `dma2` | `0x40026400` | `stm32dma` |
| `rng` | `0x50060800` | `stm32_rng` |
| `iwdg` | `0x40003000` | `stm32_independentwatchdog` |
| `rtc` | `0x40002800` | `stm32f4_rtc` |
| `rcc` | `0x40023800` | `stm32f4_rcc` |
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
| `bitbandPeripherals` | `0x42000000` | `bitbanding` |
| `bitbandSram` | `0x22000000` | `bitbanding` |

**Total Peripherals:** 49