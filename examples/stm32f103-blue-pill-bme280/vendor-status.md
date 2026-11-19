# STM32F103C8T6 Blue Pill + BME280 status

Checked live on 2026-11-19.

- ST's [STM32F103C8 product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f103c8.html)
  lists the family as **Active** and says the product is in volume production.
  Its Sample & Buy table lists order code `STM32F103C8T6` with a direct ST eStore
  link. ST's [online store](https://estore.st.com/en/products/microcontrollers-microprocessors/stm32-32-bit-arm-cortex-mcus/stm32-mainstream-mcus/stm32f1-series/stm32f103.html)
  listed the C8T6 as **Active / In Stock** when checked.
- The exact [STM32F103C8T6 Blue Pill listing at ElectroPeak](https://electropeak.com/stm32f103c8t6-arm-board)
  reported **In stock** at $2.75–$2.95 (SKU `MIC-02-012`). Blue Pill boards
  are third-party boards and clones vary; this listing does not establish that
  every board carries genuine ST silicon. The model represents the STM32F103C8T6
  reference profile, not every clone's USB, clock, or component choices. The
  listing was returned by live search on the check date; the direct product
  page timed out in the browser check.
- The [Adafruit BME280 STEMMA QT breakout](https://www.adafruit.com/product/2652)
  listed **In stock** at $14.95 and exposes I2C with level shifting for 3.3 V or
  5 V logic. The example uses address `0x77`.

The I2C pin pair follows the STM32F103C8T6 [datasheet](https://www.st.com/resource/en/datasheet/stm32f103c8.pdf):
PB6 is the default I2C1 SCL and PB7 is the default I2C1 SDA. RM0008 documents
the alternate PB8/PB9 pair as an I2C1 remap; this smoke leaves remapping disabled.
