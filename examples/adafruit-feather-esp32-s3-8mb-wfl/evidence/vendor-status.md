# Live vendor status — Adafruit ESP32-S3 Feather 8 MB w.FL (#5885)

Checked 2026-11-06 UTC.

- Adafruit's [official product listing](https://www.adafruit.com/product/5885)
  showed Product ID 5885 at **$17.50**. Its exact **8 MB Flash with w.FL**
  option was selectable with **Add to Cart**. The separate 8 MB no-PSRAM
  non-w.FL option was marked out of stock; this profile targets the in-stock
  w.FL option specifically.
- Adafruit's [ESP32-S3 Feather guide](https://learn.adafruit.com/adafruit-esp32-s3-feather/pinouts)
  identifies 8 MB flash with no PSRAM and says the post-February 2023 version
  carries the MAX17048 at I2C address 0x36.
- The guide's linked board variant pin map gives SDA GPIO3, SCL GPIO4, and
  I2C power GPIO7. The firmware enables GPIO7 before using the bus.
- Analog Devices lists MAX17048 as **RECOMMENDED FOR NEW DESIGNS**, with
  MAX17048 orderable models in **PRODUCTION** status on its live
  [product page](https://www.analog.com/en/products/max17048.html).

Demand rank: ESP32 family **#1** (38% reported MCU use). This is board coverage
within the highest-demand family, not a new vendor or a claim that this exact
board was separately ranked.
