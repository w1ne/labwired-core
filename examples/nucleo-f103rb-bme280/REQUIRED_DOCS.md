# Source documents — NUCLEO-F103RB + BME280

- ST [NUCLEO-F103RB product page](https://www.st.com/en/evaluation-tools/nucleo-f103rb.html): live board lifecycle status and distributor links.
- ST [STM32F103RB product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f103rb.html): exact MCU product lifecycle status.
- ST [NUCLEO-F103RB eStore listing](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-f103rb.html): current stock and purchase status.
- ST [UM1724 Nucleo-64 user manual](https://www.st.com/resource/en/user_manual/um1724-stm32-nucleo64-boards-mb1136-stmicroelectronics.pdf): NUCLEO-F103RB MCU, LD2/USART2 routing, and connector pinout.
- ST [DS5319 STM32F103x8/xB datasheet](https://www.st.com/resource/en/datasheet/stm32f103rb.pdf): STM32F103RB memory, package, and pin identity.
- ST [RM0008 STM32F1 reference manual](https://www.st.com/resource/en/reference_manual/cd00171190.pdf): GPIO and I2C1 register behavior.
- Bosch [BME280 datasheet](https://www.bosch-sensortec.com/media/boschsensortec/downloads/datasheets/bst-bme280-ds001.pdf): I2C operation and chip-ID register `0xD0`.
- Adafruit [BME280 STEMMA QT breakout, PID 2652](https://www.adafruit.com/product/2652): breakout wiring and pull-up/address details.
- Adafruit [BME280 Arduino library header](https://github.com/adafruit/Adafruit_BME280_Library/blob/master/Adafruit_BME280.h): primary `0x77` and alternate `0x76` I²C addresses.
- STM32Duino [F103R variant header](https://github.com/stm32duino/Arduino_Core_STM32/blob/main/variants/STM32F1xx/F103R%28C-D-E%29T/variant_generic.h) and [Wire API](https://github.com/stm32duino/Arduino_Core_STM32/wiki/API): default PB7/SDA, PB6/SCL pins and `TwoWire(sda, scl)` usage in the copy-paste sketch.
