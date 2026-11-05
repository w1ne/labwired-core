# Source documents used for the ESP32-C61-DevKitC-1 profile

- [ESP32-C61-DevKitC-1 v2.0 User Guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c61/esp32-c61-devkitc-1/user_guide.html): current N8R2 variant, GPIO11/U0TXD and GPIO10/U0RXD to the USB-UART bridge, and GPIO8 RGB LED.
- [ESP32-C61 Series Datasheet](https://documentation.espressif.com/esp32-c61_datasheet_en.html): CPU clock, memory and SoC features.
- Espressif [Arduino ESP32 board list](https://docs.espressif.com/projects/arduino-esp32/en/latest/boards/boards.html): current Arduino-core board support includes ESP32-C61.
- Espressif ESP-IDF C61 [memory map](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32c61/include/soc/soc.h) and [register bases](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32c61/register/soc/reg_base.h): HP flash/SRAM windows, UART0 and PCR addresses.
- Espressif ESP-IDF C61 [UART registers](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32c61/register/soc/uart_reg.h): UART0 FIFO and the register head shared with the C6 model.
- Espressif ESP-IDF C61 [PCR registers](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32c61/register/soc/pcr_reg.h): UART0 APB/function clock enable and source selection.
- Espressif ESP-IDF C61 [interrupt source enum](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32c61/include/soc/interrupts.h): UART0 interrupt source ordinal 40.
