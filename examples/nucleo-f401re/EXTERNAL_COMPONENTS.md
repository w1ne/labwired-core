# External Components (NUCLEO-F401RE)

No required external simulated components for minimal deterministic smoke.

The onboarding path uses on-chip peripherals only:
1. RCC
2. GPIO
3. USART2
4. SysTick

## Adding an external device (I²C / SPI sensor, EEPROM, etc.)

See [`examples/demo-blinky/`](../demo-blinky/README.md) — that example is
the canonical reference for the `external_devices` attach pattern (TMP102
on I²C1, STM32F103). The same `connection:` / `type:` / `config:` shape
works on any chip whose corresponding bus is modeled.

Before copying, check that the bus you need is actually modeled for F401:
see [`docs/boards/stm32f401.md`](../../docs/boards/stm32f401.md). At time
of writing the F401 model covers RCC / GPIO / UART2 / SysTick only — I²C,
SPI, DMA and timers are yaml-listed but not yet modeled.
