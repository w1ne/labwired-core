# Required source documents — Raspberry Pi Pico 2 W + BME280

- Raspberry Pi's [Pico 2 W product page](https://www.raspberrypi.com/products/raspberry-pi-pico-2/?variant=pico-2-w) for live availability and series lifecycle statement.
- Raspberry Pi's [Pico 2 W datasheet](https://pip-assets.raspberrypi.com/categories/1088-raspberry-pi-pico-2-w/documents/RP-008304-DS-3-pico-2-w-datasheet.pdf) for exact board identity, RP2350, I2C controllers, wireless module, and exact-board availability commitment.
- Raspberry Pi's [Pico 2 W pinout](https://pip-assets.raspberrypi.com/categories/1088-raspberry-pi-pico-2-w/documents/RP-008305-DS-1-pico-2-w-pinout.pdf) for the GP4/GP5 header routing used by this profile.
- Adafruit's [BME280 STEMMA QT breakout product page](https://www.adafruit.com/product/2652) for the external sensor listing.
- Adafruit's [BME280 breakout pinout guide](https://learn.adafruit.com/adafruit-bme280-humidity-barometric-pressure-temperature-sensor-breakout/pinouts) for its I²C pins and default address.
- Live vendor receipt: [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Peripheral model notes

The firmware and system descriptor use the existing RP2350-compatible
DesignWare I²C model and BME280 model in this repository. This profile does not
claim physical electrical or sensor parity.
