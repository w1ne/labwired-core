# STM32G431RB (Nucleo-G431RB)

Cortex-M4F, RM0440. 128 KiB flash, 22 KiB SRAM + 10 KiB CCM SRAM.

## Status at a glance

| Aspect      | Status                                                              |
|-------------|------------------------------------------------------------------------------|
| Chip yaml   | [`configs/chips/stm32g431rb.yaml`](../../configs/chips/stm32g431rb.yaml)     |
| Validation  | tier-1 fast-boot fixture (`tests/fixtures/tier1/stm32g431rb.elf`) via `examples/stm32g431rb/{tier1-smoke,io-smoke}.yaml` |
| Tier        | **structural** — tier-1 fixture only, no silicon diff                        |

## What is proven

- The tier-1 raw-register self-test boots at the G431 geometry and reports
  its clock, GPIO and UART classes PASS, then `TIER1 done`
  (`examples/stm32g431rb/io-smoke.yaml`, `examples/stm32g431rb/tier1-smoke.yaml`).
- Peripherals declared in the chip yaml reuse the stm32g474re RM0440 models:
  RCC (`stm32g4` profile), GPIOA-D (`stm32v2` profile), SysTick, USART1/2,
  LPUART1, FLASH, PWR, CRC, TIM2, TIM1 PWM, I2C1, SPI1, ADC1, DMA1, IWDG, RTC, NVIC

## What is NOT proven

- No STM32G431 silicon diff — no bench part.
- No upstream Zephyr or Arduino survival case yet; only the tier-1 fixture.
- G4-specific analog (OPAMP, COMP, DAC), FDCAN, CORDIC/FMAC and HRTIM are not modelled.

## Sources

- `configs/chips/stm32g431rb.yaml`, cross-checked against ST DS12589 and RM0440.
