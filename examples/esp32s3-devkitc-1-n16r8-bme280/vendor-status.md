# Live availability receipt — checked 2026-11-13 UTC

The target is the **ESP32-S3-DevKitC-1-N16R8** (v1.1 board), with the optional
**Adafruit BME280 STEMMA QT breakout, product 2652**.

- Espressif's [live ESP32-S3-DevKitC-1 product page](https://www.espressif.com/en/products/devkits/esp32-s3-series/esp32-s3-devkitc-1)
  describes it as an entry-level ESP32-S3-WROOM-1 board and lists the exact
  ESP32-S3-DevKitC-1-N16R8 SKU with a **Mouser** buy link.
- The [v1.1 user guide](https://documentation.espressif.com/esp-dev-kits/en/latest/esp32s3/esp32-s3-devkitc-1/user_guide_v1.1.html)
  lists N16R8 as a 16 MB flash / 8 MB PSRAM variant without an EOL mark; it
  explicitly marks N8 and N8R2 as EOL. The same guide gives the header pin map
  used here: J1-12 is GPIO8 and J1-15 is GPIO9.
- Adafruit's [live product 2652 page](https://www.adafruit.com/product/2652)
  lists the BME280 STEMMA QT breakout at $14.95, **In stock**, with Add to Cart.
  The breakout's default I2C address is 0x77.

Demand position: **ESP32 family rank 1** (38% in the JetBrains State of
Embedded 2025 survey; ESP32 kits appeared in 42% of main projects). This is
board-level sensor coverage over the existing ESP32-S3 model, not new silicon.
