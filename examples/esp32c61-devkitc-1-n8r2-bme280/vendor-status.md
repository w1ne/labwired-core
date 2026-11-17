# Live vendor and availability check

Checked **2026-11-17 UTC** for the exact ESP32-C61-DevKitC-1-N8R2 board and the
BME280 sensor.

- Espressif's [current ESP32-C61-DevKitC-1 v2.0 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c61/esp32-c61-devkitc-1/user_guide.html)
  lists `ESP32-C61-DevKitC-1-N8R2` with 8 MB flash and 2 MB PSRAM. Its J1 table
  maps pin 5 to GPIO6, pin 9 to GPIO8, and identifies the addressable RGB LED
  as driven by GPIO8; its J3 table maps pin 6 to GPIO22. Espressif does not
  assign a lifecycle label on that guide.
- The exact [Mouser N8R2 listing](https://www.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-C61-DevKitC-1-N8R2)
  showed **378 in stock**, **504 on order**, and **$12.61**. It was marked
  “New Product.”
- The live vendor guide and exact distributor listing show no NRND/EOL label;
  the distributor identifies the N8R2 SKU as a new product and available to
  ship immediately.
- Bosch's [BME280 product page](https://www.bosch-sensortec.com/en/products/environmental-sensors/humidity-sensors-bme280)
  lists I²C/SPI and states **10 years delivery**. Adafruit's [BME280 STEMMA QT
  breakout, Product 2652](https://www.adafruit.com/product/2652) showed **$14.95**
  and **In stock**.

These current product and distributor listings support that the exact board
and sensor breakout were purchasable at the check. They are not a promise of
future stock, and the simulator result is not a physical-hardware test.
