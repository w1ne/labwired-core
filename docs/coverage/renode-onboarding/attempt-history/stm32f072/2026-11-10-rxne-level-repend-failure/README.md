# STM32F072 SPI RXNE level re-pend diagnostic

This target-only run booted the exact imported `stm32f072` system for 4,000,000 instructions. The existing SPI1/SPI2 TXEIE and RXNEIE assertions passed, and masking/re-enabling RXNEIE behaved as expected. The added assertion failed because clearing NVIC pending while SPI RXNE remained asserted did not re-pend the line on either controller.

The run is preserved as a measured diagnostic, not a pass. It exposed that the NVIC ICPR path cleared an active level-source bit without re-latching it. The current `metrics/stm32f072/` result will be replaced by a new target-only run after the NVIC fix; this attempt remains visible here.
