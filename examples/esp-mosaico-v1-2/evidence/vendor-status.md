# ESP32-S31 / ESP-Mosaico V1.2 live availability

Checked 2026-11-09 UTC. This is demand-ranked ESP32 family coverage (**rank 1,
38% usage in the JetBrains State of Embedded survey**), not maintenance work.

- Espressif's [ESP32-S31 mass-production announcement](https://www.espressif.com/en/news/ESP32_S31_Mass_Production)
  says the S31 is in mass production and available for purchase.
- The official [ESP-Mosaico V1.2 user guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32s31/esp-mosaico/user_guide.html)
  identifies the board revision and documents the GPIO7 function button and
  GPIO8 vibration-motor control.
- Espressif's [official ESP-Mosaico store listing](https://store.mosaico.ai/products/esp-mosaico)
  offers the Vision Combo Kit with ESP-Mosaico Core Unit V1.2 for preorder, with
  an estimated dispatch period shown on the live product page. This is
  orderable as a preorder; it is not described here as in stock.
- Espressif's [ESP32-S31 product page](https://www.espressif.com/en/products/devkits?id=ESP32-S31)
  links to official board purchase channels.

The model's GPIO register addresses and fields follow Espressif's live
[ESP-IDF S31 GPIO register header](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32s31/register/soc/gpio_reg.h)
and [IO_MUX register header](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32s31/register/soc/io_mux_reg.h).
