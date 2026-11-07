# Live vendor status and availability

Checked 2026-11-06 UTC for M5Stack Cardputer-Adv, SKU K132-ADV.

- M5Stack's live [store listing](https://shop.m5stack.com/products/m5stack-cardputer-adv-version-esp32-s3) showed **$29.90** and **10+ In Stock**, with **Add to cart** available. The listing identifies the Stamp-S3A / ESP32-S3FN8, 8 MiB flash, and an integrated LCD.
- M5Stack's [Cardputer-Adv product guide](https://docs.m5stack.com/en/core/Cardputer-Adv) identifies the ESP32-S3FN8 at 240 MHz, 8 MB flash, and an ST7789V2 240×135 display.
- M5Stack's official [M5GFX generated board wiring](https://raw.githubusercontent.com/m5stack/M5GFX/master/src/board_detect/m5/generated/esp32s3_board_data.hpp) maps the display to GP-SPI3, GPIO36 SCK, GPIO35 MOSI, GPIO37 CS, GPIO34 D/C, GPIO33 reset, and GPIO38 backlight. It lists a 135×240 ST7789V2 panel at offsets x=52, y=40 with rotation offset 1 and inversion enabled.

Demand rank: ESP32 family **#1** (38% MCU usage in the standing demand evidence). This is board coverage in the most-used family, not a new chip. Cardputer-Adv itself is not separately ranked in that survey.
