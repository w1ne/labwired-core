# Live vendor status and availability

Checked 2026-11-05 UTC for Adafruit ESP32-S3 TFT Feather product #5483.

- Adafruit's live [product listing](https://www.adafruit.com/product/5483) showed
  **$24.95**, **100 in stock**, and **Order now to ship today**. This is the
  board being modelled; the listing includes the integrated TFT.
- Adafruit's [pinout guide](https://learn.adafruit.com/adafruit-esp32-s3-tft-feather/pinouts)
  identifies an ST7789 240×135 TFT and maps its SPI and control pins. The
  Feather uses GPIO36/GPIO35 for SPI SCK/MOSI, GPIO7 for TFT_CS, GPIO39 for
  D/C, GPIO40 for reset, GPIO45 for backlight, and GPIO21 for the shared
  TFT/STEMMA QT rail enable.
- Espressif's live [ESP32-S3 product selector](https://products.espressif.com/product-selector)
  lists the ESP32-S3 family as **Mass Production**.

Demand rank: ESP32 family **#1** (38% reported MCU use in the standing demand
evidence). This adds a board-level display path for the top-ranked family; it
does not add a new chip.
