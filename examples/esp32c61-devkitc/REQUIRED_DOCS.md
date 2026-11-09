# Source documents used for the ESP32-C61-DevKitC-1 profile

- [ESP32-C61-DevKitC-1 v2.0 User Guide](https://documentation.espressif.com/esp-dev-kits/en/latest/esp32c61/esp32-c61-devkitc-1/index.html): current N8R2 variant, GPIO4/5 at J1 pins 3/4, GPIO6 at J1 pin 5, GPIO11/U0TXD and GPIO10/U0RXD to the USB-UART bridge, and GPIO8 RGB LED.
- [ESP32-C61 Series Datasheet](https://documentation.espressif.com/esp32-c61_datasheet_en.html): CPU clock, memory and SoC features.
- Espressif [Arduino ESP32 board list](https://docs.espressif.com/projects/arduino-esp32/en/latest/boards/boards.html): current Arduino-core board support includes ESP32-C61.
- Espressif ESP-IDF C61 [memory map](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32c61/include/soc/soc.h) and [register bases](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32c61/register/soc/reg_base.h): HP flash/SRAM windows, UART0 and PCR addresses.
- Espressif ESP-IDF C61 [UART registers](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32c61/register/soc/uart_reg.h): UART0 FIFO and the register head shared with the C6 model.
- Espressif ESP-IDF C61 [PCR registers](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32c61/register/soc/pcr_reg.h): UART0 APB/function clock enable and source selection.
- Espressif ESP-IDF C61 [interrupt source enum](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32c61/include/soc/interrupts.h): UART0 interrupt source ordinal 40 and LEDC ordinal 43.
- Espressif ESP-IDF C61 [GPIO registers](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32c61/register/soc/gpio_reg.h): OUT/W1TS/W1TC, ENABLE_W1TS, GPIO8 selector at +0xAF4, and selector value 256 with OE_SEL.
- Espressif ESP-IDF C61 [IO_MUX registers](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32c61/register/soc/io_mux_reg.h): GPIO8 register at +0x20 and GPIO function 1.
- [ESP32-C61 Technical Reference Manual, Chapter 32 LEDC](https://documentation.espressif.com/esp32-c61_technical_reference_manual_en.pdf): timer/channel offsets, timer field positions, and six LEDC output-matrix signals 0–5.
- Espressif ESP-IDF C61 [LEDC registers](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32c61/register/soc/ledc_reg.h): channel and timer register definitions.
- Espressif [Arduino-ESP32 LEDC API](https://docs.espressif.com/projects/arduino-esp32/en/latest/api/ledc.html): `ledcAttach` and `ledcWrite` example API. Current C61 Arduino support requires Arduino as an ESP-IDF component or rebuilt static libraries.
- [TowerPro SG90 analog servo listing, The Pi Hut SKU 103049](https://thepihut.com/collections/towerpro/products/towerpro-servo-motor-sg90-analog): external power wiring and the buyable SG90 model used on GPIO6.
- Espressif [ESP32-C61-DevKitC-1 v2.0 schematic](https://dl.espressif.com/dl/schematics/esp32-c61-devkitc-1-schematics_v2.0.pdf): D6 is WS2812B and its data input is routed from GPIO8.
