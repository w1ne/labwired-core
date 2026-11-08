# Live vendor status: NUCLEO-U031R8 / STM32U031R8T6

Checked 2026-11-04 UTC on ST's live product and eStore pages.

- ST's [NUCLEO-U031R8 product page](https://www.st.com/en/evaluation-tools/nucleo-u031r8.html)
  reports **Active — Product is in volume production** and identifies
  STM32U031R8 as the core product.
- ST's [eStore listing](https://estore.st.com/en/nucleo-u031r8-cpn.html)
  reports **Active**, **In stock**, and **Add to cart**; the displayed price
  was **$19.60 for quantities 1–2**.
- ST's [STM32U031R8 product page](https://www.st.com/en/microcontrollers-microprocessors/stm32u031r8.html)
  reports **Active — Product is in volume production**. Its STM32U031R8T6
  variant table identifies the LQFP-64 package used by the board.
- ST's [DS14581 datasheet](https://www.st.com/resource/en/datasheet/stm32u031r8.pdf)
  identifies the U031x8 parts as up to 64 KiB Flash and 12 KiB SRAM.
- ST's [UM3256 board manual](https://www.st.com/resource/en/user_manual/um3256-stm32-nucleo64-boards-mb1932-stmicroelectronics.pdf)
  maps the default user LED LD4 to PA5; the [MB1932-U031R8-B01 schematic](https://www.st.com/resource/en/schematic_pack/mb1932-u031r8-b01-schematic.pdf)
  identifies the exact U031R8 board variant.

The onboarded model claims only the PA5 LED GPIO smoke. USART2/VCP, USB,
ST-LINK behavior, buttons, and other MCU peripherals are not represented.
