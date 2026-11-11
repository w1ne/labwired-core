# NUCLEO-G0B1RE vendor status

Checked live on 2026-11-11 against ST's product pages and eStore:

- **NUCLEO-G0B1RE:** ST lists the board as **Active — Product is in volume production**. The ST eStore listed it **In Stock**, **$20.00 for 2 units**, and offered Add to Cart.
  - https://www.st.com/en/evaluation-tools/nucleo-g0b1re.html
  - https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-g0b1re.html
- **STM32G0B1RE / STM32G0B1RET6:** ST lists the device family as **Active — Product is in volume production**; the STM32G0B1RET6 order code is marked **Active** and offers Buy Direct.
  - https://www.st.com/en/microcontrollers-microprocessors/stm32g0b1re.html
  - https://estore.st.com/en/products/microcontrollers-microprocessors/stm32-32-bit-arm-cortex-mcus/stm32-mainstream-mcus/stm32g0-series/stm32g0x1/stm32g0b1re.html

The official [UM2324 Nucleo-64 manual](https://www.st.com/resource/en/user_manual/um2324-stm32-nucleo64-boards-mb1360-stmicroelectronics.pdf) identifies the NUCLEO-G0B1RE as an MB1360 board with an STM32G0B1RET6. The repository's board descriptor uses the MB1360 schematic to map LD4 to active-high PA5 and the ST-LINK VCP to USART2 PA2/PA3. This smoke exercises those MCU paths; it does not model the ST-LINK USB bridge or assert pad-level UART waveforms.

Demand rank: **#4, other STM32 (13%)** in the MCU usage table; the STM32 Nucleo/Discovery kit category appears in **23%** of main projects. This is an exact board coverage addition to the Nucleo category, not a claim that G0B1RE itself has a separate survey percentage.
