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

## What is NOT proven

- No STM32G474 silicon diff — no bench part.
- No dedicated bare-metal or Arduino survival case, only the Zephyr boot and
  the tier-1 raw-register self-test.

## Sources

- `configs/chips/stm32g474re.yaml`, cross-checked against RM0440.
