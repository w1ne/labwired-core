# STM32G474RE (Nucleo-G474RE)

Cortex-M4, RM0440. 512 KiB flash, 128 KiB RAM.

## Status at a glance

| Aspect      | Status                                                              |
|-------------|------------------------------------------------------------------------------|
| Chip yaml   | [`configs/chips/stm32g474re.yaml`](../../configs/chips/stm32g474re.yaml)     |
| Validation  | Zephyr survival, tier-1 fast-boot fixture, and the NUCLEO-G474RE UART + LED onboarding smoke |
| Tier        | **sim-validated** — boots unmodified upstream Zephyr, no silicon diff        |

## What is proven

- UNMODIFIED upstream Zephyr `hello_world` (board `nucleo_g474re`) boots and
  prints its banner (`firmware_survival::stm32g474_zephyr`)
- The NUCLEO-G474RE onboarding smoke boots bare-metal firmware, enables the
  GPIOA and LPUART1 clock gates, prints on the default ST-LINK VCP, and records
  eight LD2/PA5 transitions. Evidence is in
  [`examples/nucleo_g474re/evidence/`](../../examples/nucleo_g474re/evidence/).
- Peripherals declared in the chip yaml: RCC (`stm32g4` profile), GPIOA-D
  (`stm32v2` profile), SysTick, USART1/2, LPUART1, FLASH, PWR, TIM2, TIM1 PWM,
  I2C1, SPI1, ADC1, DMA1, IWDG, RTC, NVIC

## What is NOT proven

- No STM32G474 silicon diff — no bench part.
- No physical UART pad waveform; the smoke captures the LPUART1 model's
  transcript.
- The smoke does not exercise USART1, I2C1, SPI1, timers, DMA, RTC, or other
  declared peripherals. ADC1 has a modeled power-up path but no conversion
  engine.

## Sources

- `configs/chips/stm32g474re.yaml`, cross-checked against RM0440.
- [UM2505, STM32G4 Nucleo-64 boards (MB1367)](https://www.st.com/resource/en/user_manual/um2505-stm32g4-nucleo64-boards-mb1367-stmicroelectronics.pdf), which identifies LD2/PA5, B1/PC13, and the default ST-LINK VCP connection on LPUART1/PA2-PA3.

## Vendor status checked 2026-11-03

ST's live [NUCLEO-G474RE page](https://www.st.com/en/evaluation-tools/nucleo-g474re.html)
states **Active** / “Product is in volume production.” The live [ST eStore
listing](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-g474re.html)
showed **In Stock** and **Add to Cart**. ST's [STM32G474RE product
page](https://www.st.com/en/microcontrollers-microprocessors/stm32g474re.html)
also states **Active** / “Product is in volume production.”
