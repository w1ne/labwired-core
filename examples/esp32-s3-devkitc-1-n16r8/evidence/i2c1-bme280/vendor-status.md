# Live vendor status — ESP32-S3-DevKitC-1 N16R8 + BME280

Checked live on 2026-11-10 UTC for the exact ESP32-S3-DevKitC-1-N16R8 board
variant and the BME280 sensor.

- Espressif's [DevKits catalog](https://www.espressif.com/en/products/devkits/esp32-s3-series/esp32-s3-devkitc-1)
  lists `ESP32-S3-DevKitC-1-N16R8` and links that SKU to Mouser. The current
  [v1.1 board guide](https://documentation.espressif.com/esp-dev-kits/en/latest/esp32s3/esp32-s3-devkitc-1/user_guide_v1.1.html)
  lists N16R8 as a current variant and marks the older N8 and N8R2 variants
  EOL. The guide exposes GPIO41 on J3 pin 7 and GPIO42 on J3 pin 6.
- Espressif's catalog links the exact [Mouser N16R8 listing](https://www.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-S3-DevKitC-1-N16R8).
  The current [Mouser Espressif stock listing](https://www.mouser.com/en/c/semiconductors/?m=Espressif)
  showed 77 in stock and 504 expected on 2026-11-10.
- Bosch's [BME280 product page](https://www.bosch-sensortec.com/en/products/environmental-sensors/humidity-sensors-bme280)
  describes the part and states a 10-year delivery commitment. Adafruit's
  [BME280 breakout PID 2652](https://www.adafruit.com/product/2652) showed
  $14.95 and an Add to Cart control today; the page confirms I²C support.
- The [ESP32-S3 Technical Reference Manual](https://documentation.espressif.com/esp32-s3_technical_reference_manual_en.pdf)
  defines I2C_EXT1 and its clock enable. This smoke routes that controller to
  the exposed GPIO41/GPIO42 pads through the GPIO matrix.

Demand rank: ESP32 family **#1** (38% of reported MCU use; 42% of kits). This is
maintenance/depth work on an existing board: it exercises the second I²C
controller and adds no new board SKU or peripheral model. The receipt is
vendor availability evidence, not a physical hardware test.
