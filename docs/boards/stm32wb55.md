# STM32WB55 (dual-core: Cortex-M4 app core + Cortex-M0+ CPU2)

512 KiB flash, 256 KiB RAM. This profile models the Cortex-M4 application
core side of the register map (RM0434), including the `stm32wb`-specific RCC
enable/reset layout, which sits at different offsets than the `stm32v2`
profile used by other STM32 parts on this simulator (see the comment in
`configs/chips/stm32wb55.yaml`).

## Status at a glance

| Aspect      | Status                                                                      |
|-------------|--------------------------------------------------------------------------------|
| Chip yaml   | [`configs/chips/stm32wb55.yaml`](../../configs/chips/stm32wb55.yaml)          |
| Validation  | `firmware_survival::stm32wb55_zephyr`                                        |
| Tier        | **sim-validated** — boots unmodified upstream Zephyr, no silicon diff        |

## What is proven

- UNMODIFIED upstream Zephyr `hello_world` boots end to end
  (`firmware_survival::stm32wb55_zephyr`), exercising the HSEM inter-core lock
  (granted to CPU1) and the RCC BDCR LSE path
- Peripherals declared in the chip yaml: RCC (`stm32wb` profile), GPIOA-D
  (`stm32v2` profile), SysTick, USART1, LPUART1, FLASH, PWR, HSEM, TIM2, TIM1
  PWM, I2C1, SPI1, ADC1, DMA1, IWDG, RTC, NVIC
- A dedicated system-memory stub at `0x1FFF0000` for the package/flash-size
  fingerprint reads Arduino/STM32Cube startup performs

## What is NOT proven

- No WB55 silicon diff — no bench part.
- No radio (BLE) modeling; CPU2 itself is not a separate simulated core — only
  the HSEM lock state CPU1's boot path depends on is modeled.
- Not covered by the tier-1 matrix (no `TIER1_TARGETS` entry).

## Sources

- `configs/chips/stm32wb55.yaml`, cross-checked against RM0434.
