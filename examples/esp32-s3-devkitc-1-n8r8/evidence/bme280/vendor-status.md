# ESP32-S3-DevKitC-1-N8R8 + BME280 availability check

Checked live on 2026-11-10:

## Board

- Espressif's current DevKits catalog lists the exact
  `ESP32-S3-DevKitC-1-N8R8` ordering code and links it to an AliExpress purchase
  page. The catalog does not show an EOL label for this listing.
- The live v1.0 user guide identifies its module as ESP32-S3-WROOM-1-N8R8,
  with 8 MB quad flash, 8 MB octal PSRAM, and 3.3 V SPI voltage, and links a
  retail sample-order path.
- The newer v1.1 user-guide table omits N8R8, while explicitly marking the N8
  and N8R2 variants EOL. It does not label N8R8 EOL. Because this differs from
  the current catalog listing, the board's live retailer link is recorded
  below as the purchase evidence; the guide discrepancy is kept visible.

Sources: [Espressif current ESP32-S3 DevKit catalog](https://www.espressif.com/en/products/devkits/esp32-s3-series/esp32-s3-devkitc-1),
[ESP32-S3-DevKitC-1 v1.0 User Guide](https://documentation.espressif.com/esp-dev-kits/en/latest/esp32s3/esp32-s3-devkitc-1/user_guide_v1.0.html),
and [ESP32-S3-DevKitC-1 v1.1 User Guide](https://documentation.espressif.com/esp-dev-kits/en/latest/esp32s3/esp32-s3-devkitc-1/user_guide_v1.1.html)
(checked live 2026-11-10).

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
