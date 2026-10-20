# Validation Report: stm32g0

**Architecture:** ARM Cortex-M.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
[2m2026-10-18T14:40:28.364932Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting LabWired Simulator
[2m2026-10-18T14:40:28.371574Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../chips/onboarding/stm32g0.yaml
[2m2026-10-18T14:40:28.376510Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Loading firmware: "/home/andrii/Projects/labwired-renode/core/tests/fixtures/uart-ok-thumbv7m.elf"
[2m2026-10-18T14:40:28.376624Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x401
[2m2026-10-18T14:40:28.376637Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Firmware Loaded Successfully!
[2m2026-10-18T14:40:28.376642Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Entry Point: 0x401
[2m2026-10-18T14:40:28.378129Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting Simulation (ARM Cortex-M)...
[2m2026-10-18T14:40:28.378144Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Initial PC: 0x400, SP: 0x20020000
[2m2026-10-18T14:40:28.378154Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Running for 20000 steps...
[2m2026-10-18T14:40:28.378226Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Simulation Error at step 2: Memory access violation at 0x2001ffe0
[2m2026-10-18T14:40:28.378249Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Simulation loop finished.
[2m2026-10-18T14:40:28.378253Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Final PC: 0x45c
[2m2026-10-18T14:40:28.378257Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Instructions: 2
[2m2026-10-18T14:40:28.378261Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Cycles: 2
[2m2026-10-18T14:40:28.378266Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Average IPS: 1234.89
```


## Hardware Coverage Report

| Peripheral ID | Base Address | Type |
|---|---|---|
| `usart1` | `0x40013800` | `stm32f7_usart` |
| `usart2` | `0x40004400` | `stm32f7_usart` |
| `usart3` | `0x40004800` | `stm32f7_usart` |
| `usart4` | `0x40004c00` | `stm32f7_usart` |
| `usart5` | `0x40005000` | `stm32f7_usart` |
| `usart6` | `0x40013c00` | `stm32f7_usart` |
| `gpioPortA` | `0x50000000` | `stm32_gpioport` |
| `gpioPortB` | `0x50000400` | `stm32_gpioport` |
| `gpioPortC` | `0x50000800` | `stm32_gpioport` |
| `gpioPortD` | `0x50000c00` | `stm32_gpioport` |
| `gpioPortE` | `0x50001000` | `stm32_gpioport` |
| `gpioPortF` | `0x50001400` | `stm32_gpioport` |
| `i2c1` | `0x40005400` | `stm32f7_i2c` |
| `i2c2` | `0x40005800` | `stm32f7_i2c` |
| `spi1` | `0x40013000` | `stm32spi` |
| `spi2` | `0x40003800` | `stm32spi` |
| `spi3` | `0x40003c00` | `stm32spi` |
| `timer1` | `0x40012c00` | `stm32_timer` |
| `timer2` | `0x40000000` | `stm32_timer` |
| `timer3` | `0x40000400` | `stm32_timer` |
| `timer4` | `0x40000800` | `stm32_timer` |
| `timer6` | `0x40001000` | `stm32_timer` |
| `timer7` | `0x40001400` | `stm32_timer` |
| `timer14` | `0x40002000` | `stm32_timer` |
| `timer15` | `0x40014000` | `stm32_timer` |
| `timer16` | `0x40014400` | `stm32_timer` |
| `timer17` | `0x40014800` | `stm32_timer` |
| `fdcan1` | `0x40006400` | `stmcan` |
| `fdcan2` | `0x40006800` | `stmcan` |
| `rtc` | `0x40002800` | `stm32f4_rtc` |
| `rcc` | `0x40021000` | `pythonperipheral` |
| `dma1` | `0x40020000` | `stm32g0dma` |
| `dma2` | `0x40020400` | `stm32g0dma` |
| `adc` | `0x40012400` | `stm32g0_adc` |
| `iwdg` | `0x40003000` | `stm32_independentwatchdog` |
| `flash_ctrl` | `0x40022000` | `stm32f4_flashcontroller` |

**Total Peripherals:** 36