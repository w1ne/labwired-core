# Source documents used for the ESP32-C5-DevKitC-1 profile

- [ESP32-C5-DevKitC-1 v1.2 User Guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c5/esp32-c5-devkitc-1/user_guide.html): N8R8 module, UART0 pins (GPIO11/GPIO12), RGB LED GPIO27, and board revision.
- [ESP32-C5-WROOM-1 & WROOM-1U Datasheet v1.3](https://documentation.espressif.com/esp32-c5-wroom-1_wroom-1u_datasheet_en.html): module configuration and current module revision.
- Espressif ESP-IDF v6.0 [C5 memory map](https://github.com/espressif/esp-idf/blob/release/v6.0/components/soc/esp32c5/include/soc/soc.h) and [register bases](https://github.com/espressif/esp-idf/blob/release/v6.0/components/soc/esp32c5/register/soc/reg_base.h): flash, HP SRAM, ROM, PCR and UART addresses.
- Espressif ESP-IDF v6.0 [C5 UART registers](https://github.com/espressif/esp-idf/blob/release/v6.0/components/soc/esp32c5/register/soc/uart_reg.h): FIFO, CLKDIV, UART register head and C6-style register tail offsets.
- Espressif ESP-IDF v6.0 [C5 PCR registers](https://github.com/espressif/esp-idf/blob/release/v6.0/components/soc/esp32c5/register/soc/pcr_reg.h): UART0 clock source selection, function-clock enable, and APB enable/reset fields.
- Espressif ESP-IDF v6.0 [C5 interrupt source enum](https://github.com/espressif/esp-idf/blob/release/v6.0/components/soc/esp32c5/include/soc/interrupts.h): UART0 source 47 and UART1 source 48.
- [Live Mouser Europe N8R8 listing](https://eu.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-C5-DevKitC-1-N8R8?qs=sGAEpiMZZMv6ooMj3OUeOcfessnCMo2lVVrnr4OuMYfdALqi8VUu1g%3D%3D): immediate stock, with the inventory snapshot recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).
- [Live DigiKey N8R8 listing](https://www.digikey.com/en/products/detail/espressif-systems/ESP32-C5-DEVKITC-1-N8R8/28718162): manufacturer lifecycle status, with its inventory snapshot recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).
