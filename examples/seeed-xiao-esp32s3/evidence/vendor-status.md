# Live vendor status — Seeed Studio XIAO ESP32-S3

Checked live on 2026-11-04 UTC:

- Seeed's [official XIAO ESP32-S3 product listing](https://www.seeedstudio.com/XIAO-ESP32S3-p-5627.html)
  identifies SKU `113991114`, lists it at **$7.49**, marks it **In stock**, and
  provides **Add to Cart**. This is the standard XIAO ESP32-S3, not the Sense or
  Plus variant.
- Seeed's [official XIAO ESP32-S3 guide](https://wiki.seeedstudio.com/xiao_esp32s3_getting_started/)
  identifies the ESP32-S3R8 and maps the user LED to GPIO21. It states the LED
  turns on when GPIO21 is low, so the modeled path is active-low.
- Espressif's [live product selector](https://products.espressif.com/product-selector)
  lists ESP32-S3 as **Mass Production**. Its [longevity commitment](https://www.espressif.com/en/products/longevity-commitment)
  lists ESP32-S3 for at least 12 years from 2021-01-01. The datasheet separately
  lists ESP32-S3R8 among the series variants and labels the distinct
  ESP32-S3R8V and ESP32-S3R2 variants EOL; the XIAO guide names ESP32-S3R8,
  not either EOL variant. See the current [ESP32-S3 datasheet v2.2](https://documentation.espressif.com/esp32-s3_datasheet_en.pdf).

The board was in stock at the check. This profile covers GPIO21 and the user
LED only; it makes no claim about wireless, PSRAM, USB, battery charging, or
other XIAO ESP32-S3 paths.
