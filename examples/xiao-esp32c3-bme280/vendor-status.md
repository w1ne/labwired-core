# Live availability receipt — checked 2026-11-13 UTC

The target is the **Seeed Studio XIAO ESP32C3** (Seeed SKU 113991054), with the
optional **Adafruit BME280 STEMMA QT breakout, product 2652**.

- Seeed's [live product page](https://www.seeedstudio.com/Seeed-XIAO-ESP32C3-p-5431.html),
  read in a real Chrome tab on 2026-11-13 (curl gets HTTP 403), lists
  "Seeed Studio XIAO ESP32-C3", SKU 113991054, **USD 4.90**, warehouse
  status **"In Stock"**, and an **"Add To Cart"** control. The page has no
  EOL or discontinued notice.
- Seeed's [XIAO ESP32C3 getting-started wiki](https://wiki.seeedstudio.com/XIAO_ESP32C3_Getting_Started/)
  gives the pin map used here: D4/SDA = GPIO6, D5/SCL = GPIO7,
  D6/TX = GPIO21, D7/RX = GPIO20. The Arduino-ESP32 3.3.x variant
  `XIAO_ESP32C3/pins_arduino.h` agrees (`SDA = 6`, `SCL = 7`).
- The Adafruit BME280 breakout (product 2652) is the same sensor part used by
  the ESP32-S3 N16R8 receipt on this branch, checked in stock on 2026-11-13.
  Its default I2C address is 0x77.

Demand position: **ESP32 family rank 1** (38% in the JetBrains State of
Embedded 2025 survey; ESP32 kits appeared in 42% of main projects). The
ESP32-C3 is the RISC-V member of that family. This is board-level sensor
coverage over the existing ESP32-C3 model, not new silicon.
