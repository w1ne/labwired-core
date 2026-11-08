# Live vendor status: NUCLEO-U385RG-Q / STM32U385RGT6Q

Checked 2026-11-04 UTC on ST's product pages and eStore.

- ST's live [NUCLEO-U385RG-Q product page](https://www.st.com/en/evaluation-tools/nucleo-u385rg-q.html)
  labels the board **Active — Product is in volume production**.
- ST's [eStore listing](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-u385rg-q.html)
  lists it as **Active** and **In Stock**, at **$25.76 for quantity 2**, with
  **Add to Cart**.
- ST's live [STM32U385RG product page](https://www.st.com/en/microcontrollers-microprocessors/stm32u385rg.html)
  labels the STM32U385 family **Active — Product is in volume production**;
  its quality table lists STM32U385RGT6Q as an active variant.
- ST's [UM3062 user manual](https://www.st.com/resource/en/user_manual/um3062-stm32u3u5-nucleo64-boards-mb1841-stmicroelectronics.pdf)
  identifies the board's default user LED LD2 on PA5 and documents its active-high
  drive through solder bridge SB10.

The model profile claims only LD2 GPIO. VCP, USB, I3C, timers, and the other MCU
peripherals remain outside this smoke profile.
