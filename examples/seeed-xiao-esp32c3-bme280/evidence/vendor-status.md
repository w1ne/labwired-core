# Live vendor status — Seeed Studio XIAO ESP32-C3 + BME280

Checked live on 2026-11-06:

- Seeed's [XIAO ESP32-C3 product page](https://www.seeedstudio.com/Seeed-XIAO-ESP32C3-p-5431.html)
  identifies SKU **113991054**, lists it at **$4.90**, and showed **In stock**
  with an **Add to Cart** control.
- Seeed's [XIAO ESP32-C3 Getting Started guide](https://wiki.seeedstudio.com/XIAO_ESP32C3_Getting_Started/)
  documents D4 as SDA/GPIO6, D5 as SCL/GPIO7, D6 as TX/GPIO21, and D7 as
  RX/GPIO20. The guide also documents the C3's 4 MB flash and 400 KB SRAM.
- Adafruit's [BME280 STEMMA QT breakout, PID 2652](https://www.adafruit.com/product/2652)
  showed its **Add to Cart** control and describes I2C operation. Its product
  page documents the 3.3 V regulator/level shifting and compatible QT/Qwiic
  connector.
- Espressif's [ESP32-C3 product page](https://www.espressif.com/en/products/socs/esp32-c3)
  lists the ESP32-C3 SoC and current product resources. The selected board's
  live Seeed listing establishes board availability; this change's descriptor
  claims only the modeled C3 blocks used by the I2C smoke.

These live listings establish current catalog availability, not physical
validation of the board or sensor.
