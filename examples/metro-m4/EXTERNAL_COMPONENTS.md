# External Components (Adafruit Metro M4)

No required external simulated components for minimal deterministic smoke.

The onboarding path uses on-chip peripherals only:
1. MCLK / GCLK
2. PORTA / PORTB
3. SERCOM3 (USART / Serial1)
4. SysTick

## On-board devices intentionally omitted

The Metro M4 Express carrier includes QSPI flash, native USB, and a NeoPixel.
Those are **not** attached in this example and are not required for the UART smoke path.

## Adding an external device (I²C / SPI sensor, EEPROM, etc.)

See [`examples/demo-blinky/`](../demo-blinky/README.md) — that example is
the canonical reference for the `external_devices` attach pattern (TMP102
on I²C1, STM32F103). The same `connection:` / `type:` / `config:` shape
works on any chip whose corresponding bus is modeled.

Before copying, check that the bus you need is actually modeled for SAMD51.
At time of writing the Metro M4 model covers MCLK/GCLK / PORT / SERCOM3 UART
/ SysTick only — additional SERCOMs, I²C/SPI modes, DMA, QSPI, and USB are not
yet modeled end-to-end.
