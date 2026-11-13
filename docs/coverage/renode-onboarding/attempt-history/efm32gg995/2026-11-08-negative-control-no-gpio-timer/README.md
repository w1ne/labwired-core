# Negative control — EFM32GG995 without the Series-0 GPIO ports and TIMERs

Same firmware (`firmware-efm32gg995-smoke`) and smoke assertions, against a
temporary copy of `configs/chips/onboarding/efm32gg995.yaml` with every
`efm32s0_gpioport` and `efm32s0_timer` entry removed (USART0 only, the
pre-round-381 descriptor).

Result: FAIL 2/5. The firmware prints its banner, then takes a precise
BusFault at 0x40006094 (GPIO port E MODEL) on its first GPIO store. The
uart_ordered, gpio_edges (gpioe:2) and memory_value (0x2001FF00) assertions
all fail, so the smoke passes only because of the new models.
