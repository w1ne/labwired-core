# ESP32-S3-DevKitC-1-N8R8 + BME280 availability check

Checked live on 2026-11-10:

## Board

- Espressif lists `ESP32-S3-DevKitC-1-N8R8` with an ESP32-S3-WROOM-1-N8R8,
  8 MB quad flash, 8 MB octal PSRAM, and 3.3 V SPI voltage in the current
  ordering table.
- The same guide gives a retail sample-order path. The N8R8 line is not marked
  EOL in the current table.

Source: [ESP32-S3-DevKitC-1 User Guide — Ordering Information and Retail
Orders](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32s3/esp-dev-kits-en-master-esp32s3.pdf)
(checked 2026-11-10; lines 157–171 in the live HTML/PDF view).

## Sensor breakout

- Bosch's live BME280 product page describes the part as "3-in-1. 10 years
  delivery" and maintains its datasheet and driver links. The page's listed
  distributors (Rutronik, Future, Arrow, Mouser, and DigiKey) currently each
  show 0 pieces / out of stock, so this is not evidence of bare-chip stock.
- The copy-paste example specifies Adafruit's BME280 STEMMA QT breakout (product
  2652). Adafruit's current product page lists it in stock at $14.95 with an
  Add to Cart control. That is the buyable item used by this example.

Sources: [Bosch BME280 product and availability](https://www.bosch-sensortec.com/en/products/environmental-sensors/humidity-sensors-bme280)
and [Adafruit BME280 STEMMA QT product 2652](https://www.adafruit.com/product/2652)
(checked 2026-11-10).

These listings support choosing the board and the named breakout for a current
hobby design. They do not establish a physical test: the committed smoke runs
against LabWired's BME280 model. The narrow board profile models the declared
8 MB flash capacity and exercised I2C0 path only; it does not model the N8R8's
PSRAM protocol.
