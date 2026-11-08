# STM32F3348-DISCO live vendor status

Checked against ST's live product and store pages on **2026-11-04 UTC**:

- ST's [STM32F3348-DISCO product page](https://www.st.com/en/evaluation-tools/32f3348discovery.html)
  lists the board as **Active** and says the product is in volume production.
- ST's [online store listing](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools-page12.html?marketing_status=23218)
  showed **STM32F3348-DISCO In Stock**, **$32.73** at the displayed 1–2 unit
  tier, with **Add to Cart** available.
- ST's [STM32F334C8 product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f334c8.html)
  lists **STM32F334C8T6 Active** and says the product is in volume production.
- ST's [STM32 MCU store listing](https://estore.st.com/en/products/microcontrollers-microprocessors/stm32-mainstream-mcus-page110.html?marketing_status=23218&p=109)
  showed **STM32F334C8T6TR In Stock**.
- ST's [UM1735 Rev 3](https://www.st.com/resource/en/user_manual/um1735--discovery-kit-for-stm32f3-series-with-stm32f334c8-mcu-stmicroelectronics.pdf)
  maps the red user LED LD3 to PB6. The [STM32F3348-Discovery BSP source](https://documentation.help/STM32F3348-Discovery-BSP-Drivers/documentation.pdf)
  implements `BSP_LED_On` by writing a high level, so LD3 is modeled as active
  high.

The profile covers only the STM32F334C8T6 GPIOB/PB6 path needed by this smoke.
Other onboard LEDs, the button, VCP, and high-brightness LED driver are not
claimed. Vendor status and stock can change after this check.
