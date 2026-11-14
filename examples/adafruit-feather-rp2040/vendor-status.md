# Live availability — Adafruit Feather RP2040 + BME280

Checked 2026-11-11 UTC:

- Adafruit's official [Feather RP2040 PID 4884 listing](https://www.adafruit.com/product/4884)
  showed **$11.95, In stock, Add to Cart**. The listing identifies the RP2040,
  264 KB SRAM, and 8 MB SPI flash. Raspberry Pi's [RP2040 specifications](https://www.raspberrypi.com/products/rp2040/specifications/)
  say the part is expected to remain in production until at least January 2041.
- Adafruit's official [BME280 STEMMA QT PID 2652 listing](https://www.adafruit.com/product/2652)
  showed **$14.95, In stock, Add to Cart**. This smoke uses the assembled
  breakout, not a claim about stock of the bare BME280 package.
- Adafruit's [Feather RP2040 pinout guide](https://learn.adafruit.com/adafruit-feather-rp2040-pico/pinouts)
  identifies the STEMMA QT connector as I2C1 on SCL/GP3 and SDA/GP2, and names
  `Wire` as the Arduino API for that connector.

## Modeled scope

The system profile covers RP2040 I2C1, the STEMMA QT BME280 at address `0x77`,
and the chip-ID read from register `0xD0`. It does not claim coverage of the
onboard NeoPixel, boot/reset buttons, USB CDC, battery charging, or analog
voltage behavior. The board's 8 MiB QSPI flash is described as directly mapped
firmware memory; QSPI electrical behavior and the flash chip's command path are
not part of this smoke.
