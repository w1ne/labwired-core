# NUCLEO-H503RB live vendor status

Checked live on 2026-11-11:

- **STM32H503RB:** ST marks it **Active — Product is in volume production**.
  The product page lists a 250 MHz Cortex-M33, 128 KB flash, and 32 KB RAM.
  https://www.st.com/en/microcontrollers-microprocessors/stm32h503rb.html
- **NUCLEO-H503RB:** ST marks it **Active — Product is in volume production**.
  ST's eStore lists it **In Stock** at **$26.35 / 2 units** with Add to Cart.
  https://www.st.com/en/evaluation-tools/nucleo-h503rb
  https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-h503rb.html

Board wiring comes from ST UM3121 (MB1814). It identifies NUCLEO-H503RB as
the STM32H503RBT6 variant, supports Arduino Uno V3 headers, maps the green LD2
to active-high PA5, and maps B1 to PC13 by default. For the H503RB default
solder-bridge position, Table 13 routes the STLINK-V3EC VCP through USART3 on
PA3/PA4. The simulator captures USART3 register output; it does not emulate
the ST-LINK bridge or validate pad-level waveforms.
https://www.st.com/resource/en/user_manual/um3121-stm32h5-nucleo64-board-mb1814-stmicroelectronics.pdf

This profile only claims GPIOA/GPIOC, the H5 RCC, USART3, SysTick, LD2, B1,
and the default VCP selection. USB, ST-LINK, other peripherals and board
connectors are not modeled. GPIO/USART behavior is simulator-derived and has
not been compared to a physical H503 board.
