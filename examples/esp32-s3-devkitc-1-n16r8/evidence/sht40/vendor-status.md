# Live vendor availability — ESP32-S3-DevKitC-1 N16R8 + SHT40

Checked live on 2026-11-10 UTC for the exact board variant and the sensor
breakout.

- Espressif's current [ESP32-S3-DevKitC-1 v1.1 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32s3/esp32-s3-devkitc-1/user_guide_v1.1.html)
  lists ESP32-S3-DevKitC-1-N16R8 as a current ordering variant. It marks the
  older N8 and N8R2 variants EOL. The guide maps J3 pin 7 to GPIO41 and J3 pin
  6 to GPIO42.
- Espressif's live [DevKits catalog](https://www.espressif.com/en/products/devkits/esp32-s3-series/esp32-s3-devkitc-1)
  links the exact N16R8 ordering SKU to Mouser. Mouser's [N16R8 listing](https://www.mouser.com/ProductDetail/Espressif-Systems/ESP32-S3-DevKitC-1-N16R8)
  showed **77 in stock** at **US$13.24** per unit, with another 504 expected
  2026-12-07.
- Adafruit's current [SHT40 STEMMA QT / Qwiic breakout page, PID 4885](https://www.adafruit.com/product/4885)
  showed **US$5.95** and an enabled Add to Cart control. It lists I²C address
  0x44, 3.3–5 V breakout supply, and onboard pull-up resistors.

Demand rank: ESP32 family **#1** (38% MCU usage and 42% of kits in the measured
JetBrains survey). This is coverage depth on the existing DevKitC board and
reuses its I²C1 model plus the existing SHT40 model; it adds a sensor use case,
not a new MCU or board SKU. These are vendor availability checks, not physical
hardware evidence.
