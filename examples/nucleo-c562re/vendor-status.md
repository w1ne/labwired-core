# Vendor status: NUCLEO-C562RE

Checked live on **2026-11-11** against ST's product and store pages:

- [ST eStore — NUCLEO-C562RE](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-c562re.html): **Active**, **In Stock**, **$21.87**, with Add to Cart available.
- [ST product page — NUCLEO-C562RE](https://www.st.com/en/evaluation-tools/nucleo-c562re.html): **Active**, in volume production.
- [ST product page — STM32C562RE](https://www.st.com/en/microcontrollers-microprocessors/stm32c562re.html): **Active**, in volume production; Cortex-M33 up to 144 MHz, 512 KiB flash, 128 KiB SRAM.

Board wiring comes from [UM3615, NUCLEO-C562RE user manual](https://www.st.com/resource/en/user_manual/um3615-stm32c5-nucleo64-board-mb2213-stmicroelectronics.pdf): the board uses STM32C562RET6; LD1 is active-high PA5; the default ST-LINK VCP is USART2 on PA2/PA3 with SB12/SB26 on and SB46 off. The smoke profile reflects only those exercised paths.
