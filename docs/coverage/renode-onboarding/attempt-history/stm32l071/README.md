# STM32L071 attempt history

`attempt-01/` preserves the first measured run against the narrowed imported
profile. The Cortex-M0+ firmware executed 800,000 instructions, emitted its
USART2 banner, and stopped at `max_steps`, but the PA5 GPIO-edge assertion
failed because the generic `stm32_gpioport` declaration selected the default
STM32F1 register layout. The profile was corrected to explicitly select the
modeled `stm32v2` GPIO layout. The subsequent run passed all three smoke
assertions; its current evidence is under `metrics/stm32l071/`.

The 64 KiB flash and 20 KiB SRAM values are execution windows in this profile.
The exact STM32L071 density variant was not resolved through Part Knowledge,
so these values are not presented as a source-verified full-device memory map.
