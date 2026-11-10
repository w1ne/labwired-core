# Live vendor status — NUCLEO-F103RB + BME280

Checked live on **2026-11-10 UTC**.

- ST's [NUCLEO-F103RB product page](https://www.st.com/en/evaluation-tools/nucleo-f103rb.html)
  lists the board as **Active** and says the product is in volume production.
  Its distributor table reports the NUCLEO-F103RB available from five
  distributors: DigiKey 1,600, Mouser 1,263, Farnell 3,432, Newark 216, and
  Future Electronics 20. ST labels that inventory snapshot **2026-10-25**.
- ST's [NUCLEO-F103RB eStore listing](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-f103rb.html)
  showed **Active**, **In Stock**, and **Add to Cart** when checked. It listed
  $32.82 at the 2-unit tier.
- ST's [STM32F103RB product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f103rb.html)
  marks the exact family **Active / Product is in volume production** and lists
  orderable `STM32F103RBT6` variants as Active. The Nucleo-64 user manual says
  NUCLEO-F103RB ships the STM32F103RBT6.
- Adafruit's [BME280 STEMMA QT breakout listing](https://www.adafruit.com/product/2652)
  identifies PID 2652, listed $14.95, and showed **Add to Cart** when checked.
  Adafruit's current [BME280 library header](https://github.com/adafruit/Adafruit_BME280_Library/blob/master/Adafruit_BME280.h)
  identifies `0x77` as the primary I²C address and `0x76` as the alternate.
  Bosch's [BME280 product page](https://www.bosch-sensortec.com/en/products/environmental-sensors/humidity-sensors-bme280)
  lists I²C and SPI interfaces.

Demand: STM32F103 **#3** (17% of reported MCU use); STM32 Nucleo/Discovery
boards **23%** of reported kit use. This is a sensor/system extension to the
existing NUCLEO-F103RB model, not a new chip pick.
