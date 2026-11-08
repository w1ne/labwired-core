# Live vendor status and availability

Checked 2026-11-04 UTC:

- Seeed's [official XIAO ESP32-C6 product page](https://www.seeedstudio.com/Seeed-Studio-XIAO-ESP32C6-p-5884.html)
  lists SKU `113991254` at **$5.20**, marks it **In stock**, and provides
  **Add to Cart**. The board page identifies the design as based on Espressif's
  ESP32-C6.
- Seeed's [official XIAO ESP32-C6 guide](https://wiki.seeedstudio.com/xiao_esp32c6_getting_started/)
  documents the yellow user light on GPIO15 and the board's ESP32-C6FH4 SoC.
  The [official board schematic](https://files.seeedstudio.com/wiki/SeeedStudio-XIAO-ESP32C6/XIAO_ESP32_C6_v1.0_SCH_260114.pdf)
  shows the LED circuit connected to GPIO15; the board's USER LED is active-low.
- Espressif's [ESP32-C6 series datasheet v1.5](https://documentation.espressif.com/esp32-c6_datasheet_en.html)
  covers ESP32-C6FH4 and states that datasheet version 1.0 and later is the
  official release for products in mass production. No NRND or EOL marking was
  present in the live product material checked.

The stock check is for the complete board; lifecycle evidence is for the
on-board ESP32-C6 silicon. The smoke profile covers GPIO15 only. Wireless,
USB, battery charging/measurement, the LP core, ADC, UART, I2C, and SPI are not
claimed as working board paths here.
