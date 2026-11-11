# NUCLEO-H533RE vendor status

Checked live on 2026-11-10 against ST's product pages:

- **STM32H533RET6:** ST marks the part **Active — Product is in volume production**. The product page lists 512 KB flash, 272 KB RAM, and a 250 MHz Cortex-M33.
  https://www.st.com/en/microcontrollers-microprocessors/stm32h533re.html
- **NUCLEO-H533RE:** ST marks the board **Active — Product is in volume production**. The ST eStore lists it **In Stock** at **$25.69 / 2 units** and offers Add to Cart.
  https://www.st.com/en/evaluation-tools/nucleo-h533re
  https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-h533re.html

Board wiring is taken from ST UM3121 (MB1814): LD2 is PA5, active high, and
the user button B1 is PC13, active high by default. UM3121 Table 13's H533RE
row identifies the default STLINK-V3EC virtual COM route as USART2 on PA2/PA3;
the simulator captures the USART2 register model and does not emulate the
STLINK bridge or the pad-level waveform.
https://www.st.com/resource/en/user_manual/um3121-stm32h5-nucleo64-board-mb1814-stmicroelectronics.pdf
