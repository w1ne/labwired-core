# Live vendor status — NUCLEO-G474RE / STM32G474RET6

Checked 2026-11-11 UTC.

- ST's live [STM32G474RE product page](https://www.st.com/en/microcontrollers-microprocessors/stm32g474re.html)
  lists **STM32G474RET6 — Active** and says the product is in volume production.
- ST's [NUCLEO-G474RE store page](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-g474re.html)
  lists the board as **Active**.
- DigiKey's [live NUCLEO-G474RE listing](https://www.digikey.com/en/products/detail/stmicroelectronics/NUCLEO-G474RE/10231585)
  showed **2,262 in stock** and product status **Active** at the time checked.

This check applies to STM32G474RET6 and NUCLEO-G474RE. It does not imply that
every STM32G474 package or sibling board has the same lifecycle status.

Pin/function references:

- ST's [STM32G474 datasheet](https://www.st.com/resource/en/datasheet/dm00431551.pdf)
  assigns PA8 to TIM1_CH1 on AF6.
- ST's [STM32CubeG4 NUCLEO-G474RE TIM complementary-signals example](https://github.com/STMicroelectronics/STM32CubeG4/tree/master/Projects/NUCLEO-G474RE/Examples/TIM/TIM_ComplementarySignals)
  uses PA8 for TIM1_CH1 on the physical board.
