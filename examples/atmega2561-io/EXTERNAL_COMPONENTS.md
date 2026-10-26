# External Components — ATmega2561

None. This is a bare 64-pin part, not a board. The example binds the model's
PB5 pad to a status LED in `system.yaml`; no external device is attached to
the SPI, I2C, USART or ADC windows.

The descriptor keeps the parking `spi` and `i2c` controllers so kits can be
attached later, but the smoke uses only CPU-owned GPIO and USART0.
