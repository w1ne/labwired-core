# Required source documents

- ST [NUCLEO-F411RE product page](https://www.st.com/en/evaluation-tools/nucleo-f411re.html): board status and product family.
- ST [UM1724, STM32 Nucleo-64 boards](https://www.st.com/resource/en/user_manual/um1724-stlinkv21-in-circuit-debuggerprogrammer-for-stm8-and-stm32-stmicroelectronics.pdf): Nucleo-64 board wiring and VCP.
- ST [STM32F411RE data brief](https://www.st.com/resource/en/data_brief/nucleo-f411re.pdf) and [STM32F411xC/xE datasheet](https://www.st.com/resource/en/datasheet/stm32f411re.pdf): MCU and memory profile.
- STM32duino's [NUCLEO-F411RE variant](https://github.com/stm32duino/Arduino_Core_STM32/blob/main/variants/STM32F4xx/F411R%28C-E%29T/variant_NUCLEO_F411RE.h) maps D14/D15 to PB9/PB8; the Arduino core's [`pins_arduino.h`](https://github.com/stm32duino/Arduino_Core_STM32/blob/main/cores/arduino/pins_arduino.h) selects those digital pin numbers as the default `Wire` connection.
- Adafruit [BME280 STEMMA QT breakout, PID 2652](https://www.adafruit.com/product/2652) and its [product guide](https://learn.adafruit.com/adafruit-bme280-humidity-barometric-pressure-temperature-sensor-breakout).
- Bosch [BME280 datasheet](https://www.bosch-sensortec.com/media/boschsensortec/downloads/datasheets/bst-bme280-ds002.pdf).
