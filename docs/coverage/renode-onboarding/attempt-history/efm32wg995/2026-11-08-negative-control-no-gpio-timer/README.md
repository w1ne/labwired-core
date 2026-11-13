# Negative control — EFM32WG995 without the Series-0 GPIO ports and TIMERs

Same firmware (`firmware-efm32wg995-smoke`, shared `s0.rs` checks over UART0)
and smoke assertions, against a temporary copy of the pre-round-382
`configs/chips/onboarding/efm32wg995.yaml` (UART0 only, no
`efm32s0_gpioport` / `efm32s0_timer` entries).

Result: FAIL 2/5. The firmware prints its banner, then takes a precise
BusFault at 0x40006094 (GPIO port E MODEL) on its first GPIO store. The
uart_ordered, gpio_edges (gpioe:2) and memory_value (0x20007F00) assertions
all fail, so the smoke passes only because of the new descriptor entries.
