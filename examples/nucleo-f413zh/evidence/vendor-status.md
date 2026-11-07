# Live vendor status: NUCLEO-F413ZH

Checked 2026-11-04 against ST's live product and eStore pages.

- [ST NUCLEO-F413ZH product page](https://www.st.com/en/evaluation-tools/nucleo-f413zh.html)
  lists the board as **Active** and says it is in volume production.
- [ST eStore listing for NUCLEO-F413ZH](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-f413zh.html)
  showed **Active**, **In Stock**, **$30.03 at 2 units**, and an **Add to Cart** option.
- [ST eStore listing for STM32F413ZHT6](https://estore.st.com/en/stm32f413zht6-cpn.html)
  showed the MCU as **Active** and **In stock**.

## Hardware references

- [ST UM1974, STM32 Nucleo-144 boards](https://www.st.com/resource/en/user_manual/dm00244518-stm32-nucleo144-boards-stmicroelectronics.pdf), §7.5: blue user LED LD2 is connected to PB7 and is active-high.
- [ST STM32F413ZH datasheet](https://www.st.com/resource/en/datasheet/stm32f413zh.pdf): STM32F413ZHT6 has 1.5 MiB Flash and 320 KiB SRAM.
- [ST RM0430, STM32F413/423 reference manual](https://www.st.com/resource/en/reference_manual/dm00305666-stm32f413-423-advanced-arm-based-32-bit-mcus-stmicroelectronics.pdf): SRAM1 is 256 KiB at `0x20000000`; the separate SRAM2 window is intentionally omitted from this narrow profile.
