# Required source material

- [ESP32-H2-DevKitM-1 current guide](https://documentation.espressif.com/esp-dev-kits/en/latest/esp32h2/esp32-h2-devkitm-1/index.html): exact N4S product variant, board header pins, and USB-to-UART wiring.
- [ESP32-H2 Technical Reference Manual v1.1](https://www.espressif.com/sites/default/files/documentation/esp32-h2_technical_reference_manual_en.pdf): I2C controller and GPIO matrix behavior.
- [ESP-IDF v5.4.1 H2 I2C register header](https://github.com/espressif/esp-idf/blob/v5.4.1/components/soc/esp32h2/register/soc/i2c_reg.h): I2C0 register offsets.
- [ESP-IDF v5.4.1 H2 GPIO register header](https://github.com/espressif/esp-idf/blob/v5.4.1/components/soc/esp32h2/register/soc/gpio_reg.h): GPIO output/input selector register windows.
- [ESP-IDF v5.4.1 H2 GPIO signal map](https://github.com/espressif/esp-idf/blob/v5.4.1/components/soc/esp32h2/include/soc/gpio_sig_map.h): I2CEXT0 SCL/SDA matrix signal IDs 45/46.
- [ESP-IDF v5.4.1 H2 interrupt source table](https://github.com/espressif/esp-idf/blob/v5.4.1/components/soc/esp32h2/include/soc/interrupts.h): I2C_EXT0 interrupt source 39.
- [Espressif ESP DevKits catalog](https://www.espressif.com/products/devkits) and [DigiKey exact N4S listing](https://www.digikey.com/en/products/detail/espressif-systems/ESP32-H2-DEVKITM-1-N4S/26282483): current purchasability snapshot; see [vendor-status.md](vendor-status.md).
