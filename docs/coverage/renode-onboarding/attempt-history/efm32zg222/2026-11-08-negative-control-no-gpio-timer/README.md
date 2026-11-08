# Negative control — EFM32ZG222 without the Series-0 GPIO ports and TIMERs

Same firmware (`firmware-efm32zg222-smoke`, shared `s0.rs` checks over
USART1) and smoke assertions, against a temporary copy of the pre-round-383
`configs/chips/onboarding/efm32zg222.yaml` (USART1 only, no
`efm32s0_gpioport` / `efm32s0_timer` entries).

Result: FAIL 2/5. The firmware prints its banner, then faults at 0x40006050
(GPIO port C MODEH) on its first GPIO access. The uart_ordered, gpio_edges
(gpioc:10) and memory_value (0x20000F00) assertions all fail, so the smoke
passes only because of the new descriptor entries.

Note: the fault report prints CFSR/BFAR even though this is a Cortex-M0+
(ARMv6-M has no configurable fault status registers; silicon just takes
HardFault). The engine's diagnostic, not the firmware's view.
