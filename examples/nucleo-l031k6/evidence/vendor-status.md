# NUCLEO-L031K6 live vendor status

Checked 2026-11-04 UTC against ST's live product pages and ST Online Store:

- ST's [NUCLEO-L031K6 product page](https://www.st.com/en/evaluation-tools/nucleo-L031k6.html)
  lists the board as **Active** and says it is in volume production.
- ST's [NUCLEO-L031K6 eStore listing](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-l031k6.html)
  showed **In Stock**, **$16.02 / 2 units**, and **Add to Cart**.
- ST's [STM32L031K6 product page](https://www.st.com/en/microcontrollers-microprocessors/stm32l031k6.html)
  lists the MCU family as **Active** and in volume production; its product table
  lists the STM32L031K6T6 orderable variant as Active.
- ST's [UM1956 Rev 6](https://www.st.com/resource/en/user_manual/um1956-stm32-nucleo32-boards-mb1180-stmicroelectronics.pdf)
  §6.5 and Table 14 identify the green user LED LD3 on PB3, routed to Arduino
  Nano D13 at CN4.15. This profile models only that GPIO path.

The board was directly orderable from ST at this check. Inventory and price can
change after the recorded check.
