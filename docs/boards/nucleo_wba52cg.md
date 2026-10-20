# Validation Report: nucleo_wba52cg

**Architecture:** ARMv8-M.

## 1. Dynamic Simulation Validation
**Status:** ✅ Passed (simulation-ok)

```text
[2m2026-10-18T14:40:07.705339Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting LabWired Simulator
[2m2026-10-18T14:40:07.710515Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../chips/onboarding/nucleo_wba52cg.yaml
[2m2026-10-18T14:40:07.725682Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Loading firmware: "/home/andrii/Projects/labwired-renode/core/tests/fixtures/uart-ok-thumbv7m.elf"
[2m2026-10-18T14:40:07.725777Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x401
[2m2026-10-18T14:40:07.725791Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Firmware Loaded Successfully!
[2m2026-10-18T14:40:07.725795Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Entry Point: 0x401
[2m2026-10-18T14:40:07.726814Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Starting Simulation (ARM Cortex-M)...
[2m2026-10-18T14:40:07.726831Z[0m [32m INFO[0m [2mlabwired_cli::commands::run[0m[2m:[0m Initial PC: 0x400, SP: 0x20020000
[2m2026-10-18T14:40:07.726841Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Running for 20000 steps...
[2m2026-10-18T14:40:07.735045Z[0m [32m INFO[0m [2mlabwired_cli[0m[2m:[0m Progress: 10000 steps, current IPS: 1082656.61
[2m2026-10-18T14:40:07.742044Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Simulation loop finished.
[2m2026-10-18T14:40:07.742140Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Final PC: 0x466
[2m2026-10-18T14:40:07.742180Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Instructions: 19999
[2m2026-10-18T14:40:07.742212Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Total Cycles: 20002
[2m2026-10-18T14:40:07.742242Z[0m [32m INFO[0m [2mlabwired_cli::report[0m[2m:[0m Average IPS: 1216695.33
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