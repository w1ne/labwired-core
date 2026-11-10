# Live vendor status — STM32F103C8T6 Blue Pill + BME280

Checked live on **2026-11-10 UTC**.

- ST's [STM32F103C8 product and store page](https://estore.st.com/en/products/microcontrollers-microprocessors/stm32-32-bit-arm-174-cortex-174-mcus/stm32-mainstream-mcus/stm32f1-series/stm32f103/stm32f103c8.html)
  lists the exact `STM32F103C8T6` as **Active** and **In Stock**, with an
  **Add to Cart** control. ST's current eStore showed $4.03 per unit at the
  500-piece tier. The current ST datasheet says the STM32F103x8/xB line is in
  full production.
- Sangar Tech's [exact Blue Pill board listing](https://www.sangartech.com/products/stm32f103c8t6-blue-pill-development-board)
  identifies the board's MCU as `STM32F103C8T6` and showed **$3.00**, **In
  stock**, and **Add to cart**.
- Adafruit's [BME280 STEMMA QT breakout page](https://www.adafruit.com/product/2652)
  identifies **PID 2652**, listed **$14.95**, **In stock**, and **Add to Cart**.
  The matching [Bosch BME280 product page](https://www.bosch-sensortec.com/en/products/environmental-sensors/humidity-sensors-bme280)
  lists I²C and SPI interfaces and states a 10-year delivery commitment.

The generic Blue Pill layout follows the linked schematic for its active-low
PC13 LED and default I2C1 pins PB6/PB7. Blue Pill boards can vary by supplier;
this profile claims only the exact C8T6 memory/package geometry and the pins
included in this smoke.
