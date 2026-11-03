# External Components (NUCLEO-F401RE)

No required external simulated components for minimal deterministic smoke.

The onboarding smoke uses on-chip RCC, GPIOA5, and USART2. It does not attach
or exercise an external simulated component.

## Adding an external device (I²C / SPI sensor, EEPROM, etc.)

See [`examples/demo-blinky/`](../demo-blinky/README.md) — that example is
the canonical reference for the `external_devices` attach pattern (TMP102
on I²C1, STM32F103). The same `connection:` / `type:` / `config:` shape
works on any chip whose corresponding bus is modeled.

Before copying, check that the bus and device behavior you need are modeled
for F401; see [`docs/boards/stm32f401.md`](../../docs/boards/stm32f401.md).
The F401RE descriptor declares I2C1 and SPI1, but this board smoke does not
exercise either bus or a sensor model.
