# Failed HCLK/8 smoke attempt

The imported STM32F072 platform booted and executed 4,000,000 instructions. The existing checks passed, but the new `systick clock-source` UART assertion failed because `main` did not yet invoke `check_systick_clock_sources()`. This is a smoke wiring error; the run produced no clock-source observation. The next attempt adds the missing call and will rerun this one target.
