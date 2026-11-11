# NUCLEO-H753ZI live vendor status

Checked live on 2026-11-11 UTC:

- ST marks **NUCLEO-H753ZI Active — Product is in volume production** on its
  [official product page](https://www.st.com/en/evaluation-tools/nucleo-h753zi.html).
- ST's [eStore listing](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-h753zi.html)
  showed **In Stock**, **$40.26 / 2 units**, and **Add to Cart**.
- ST marks the fitted **STM32H753ZIT6 Active — Product is in volume
  production** on its [STM32H753ZI page](https://www.st.com/en/microcontrollers-microprocessors/stm32h753zi.html).

Board wiring is from ST UM2407, the NUCLEO-H7 Nucleo-144 user manual. By
default, LD1 is active-high PB0, B1 is connected to PC13, and the ST-LINK VCP
uses USART3 TX/RX on PD8/PD9. The smoke tests the GPIOB PB0 output and USART3
register console. It does not emulate the ST-LINK bridge or verify pad-level
signals.

The current descriptor only claims GPIOB/C/D, USART3, RCC, SysTick, the
LD1/B1 bindings, and the listed memory windows. The other user LEDs, Ethernet,
USB, board connectors, and other H753 peripheral blocks are not modeled by
this profile. No physical-board capture is claimed.

Sources:

- [ST NUCLEO-H753ZI product page](https://www.st.com/en/evaluation-tools/nucleo-h753zi.html)
- [ST STM32H753ZI product page](https://www.st.com/en/microcontrollers-microprocessors/stm32h753zi.html)
- [ST UM2407 NUCLEO-H7 Nucleo-144 user manual](https://www.st.com/resource/en/user_manual/dm00499160-stm32h7-nucleo-144-boards-mb1364-stmicroelectronics.pdf)
- [ST DS12117 STM32H753xI datasheet](https://www.st.com/resource/en/datasheet/stm32h753zi.pdf)
- [ST RM0433 STM32H743/753 reference manual](https://www.st.com/resource/en/reference_manual/rm0433-stm32h742-stm32h743753-and-stm32h750-value-line-advanced-armbased-32bit-mcus-stmicroelectronics.pdf)
