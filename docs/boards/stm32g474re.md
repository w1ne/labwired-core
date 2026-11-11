# STM32G474RE (Nucleo-G474RE)

Cortex-M4, RM0440. 512 KiB flash, 128 KiB RAM.

## Status at a glance

| Aspect      | Status                                                              |
|-------------|------------------------------------------------------------------------------|
| Chip yaml   | [`configs/chips/stm32g474re.yaml`](../../configs/chips/stm32g474re.yaml)     |
| Validation  | `firmware_survival::stm32g474_zephyr`; tier-1 fast-boot fixture (`tests/fixtures/tier1/stm32g474re.elf`) |
| Tier        | **sim-validated** — boots unmodified upstream Zephyr, no silicon diff        |

## What is proven

- UNMODIFIED upstream Zephyr `hello_world` (board `nucleo_g474re`) boots and
  prints its banner (`firmware_survival::stm32g474_zephyr`)
- Peripherals declared in the chip yaml: RCC (`stm32g4` profile), GPIOA-D
  (`stm32v2` profile), SysTick, USART1/2, LPUART1, FLASH, PWR, TIM2, TIM1 PWM,
  I2C1, SPI1, ADC1, DMA1, IWDG, RTC, NVIC
- TIM1_CH1 single-ended PWM reaches PA8 through the G4 AF6 map. The committed
  [Nucleo PWM smoke](../../examples/nucleo_g474re/README.md) checks the generated
  pad transitions, not only register state.

## What is NOT proven

- No silicon diff — the timer waveform evidence is model-derived.
- TIM1 complementary outputs, dead-time insertion, break inputs, and
  center-aligned mode are not covered by the single-ended edge-aligned smoke.

## Sources

- `configs/chips/stm32g474re.yaml`, cross-checked against RM0440.
