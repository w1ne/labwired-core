# External Components (Adafruit Feather STM32F405 Express)

No required external simulated components for minimal deterministic smoke.

The board smoke uses the on-chip GPIO controller for D13/PC1 and USART3 on
PB10/PB11. No external device is attached in the smoke system.

The board's integrated NeoPixel, 2 MB SPI flash, microSD/SDIO slot, USB-C data
path, and LiPo charging circuit are not represented as working board devices
in this profile. A chip descriptor having an SPI or SDIO block does not model
the board-level device or its wiring.

## Adding an external device (I²C / SPI sensor, EEPROM, etc.)

See [`examples/demo-blinky/`](../demo-blinky/README.md) — that example is
the canonical reference for the `external_devices` attach pattern (TMP102
on I²C1, STM32F103). The same `connection:` / `type:` / `config:` shape
works on any chip whose corresponding bus is modeled.

Before adding a component, check that the required bus instance is declared
and modeled for F405: see [`docs/boards/stm32f405.md`](../../docs/boards/stm32f405.md).
