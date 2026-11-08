# Live status and availability check

Checked 2026-11-06 UTC for the ESP32-S3-DevKitC-1 N16R8 and BME280 breakout.

- Espressif's [current ESP32-S3 DevKit catalog](https://www.espressif.com/en/products/devkits/esp32-s3-series/esp32-s3-devkitc-1)
  lists `ESP32-S3-DevKitC-1-N16R8` and links the exact variant to Mouser.
  Espressif's [current board guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32s3/esp32-s3-devkitc-1/index.html)
  says the DevKitC-1 exposes most module I/O pins for jumper wiring. The
  catalog does not display an NRND/EOL label for this variant.
- Wavtron's live [ESP32-S3-DevKitC N16R8 listing](https://www.wavtron.in/products/esp32-s3-devkitc-n16r8-dev-board)
  showed **In Stock**, ₹760, and **Add to Cart**. It identifies the N16R8
  configuration; this retailer page does not distinguish PCB revision.
- Adafruit's live [BME280 STEMMA QT product page](https://www.adafruit.com/product/2652)
  showed **In stock**, $14.95, and an Add to Cart control. The product page
  identifies the breakout as supporting I²C or SPI.

Demand rank: ESP32 family **#1** in the standing evidence (38% MCU use; 42%
ESP32 kits). This is an extension of an existing board profile, adding a
modeled I²C0 sensor path; it does not add a new MCU family or peripheral model.
