# STM32F103R6T6A live vendor status

Checked live on **2026-11-17 UTC** for the exact order code represented by this
LQFP64 profile, `STM32F103R6T6A`.

- ST's [STM32F103R6 product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f103r6.html)
  lists `STM32F103R6T6A` as **Active** and the product family as in volume
  production.
- ST's [official eStore page for STM32F103R6T6A](https://estore.st.com/en/stm32f103r6t6a-cpn.html)
  lists the part as **Active** and **In stock**, with **Add to cart** available.
  It identifies the package as LQFP64.
- ST's [STM32F103R6 datasheet](https://www.st.com/resource/en/datasheet/stm32f103r6.pdf)
  is the geometry source for the 32 KB Flash / 10 KB SRAM, 51 GPIO, 16 ADC
  channel profile and the package pinout used by the smoke.
- ST's [RM0008 reference manual](https://www.st.com/resource/en/reference_manual/rm0008-stm32f103xx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf)
  supplies the register map and peripheral bases; [ES0348](https://www.st.com/resource/en/errata_sheet/es0348-stm32f101x46-stm32f102x46-stm32f103x46-device-errata-stmicroelectronics.pdf)
  §2.2.2 documents the user-mode `DBGMCU_IDCODE` read restriction reflected
  in the descriptor.

This is a bare MCU profile and simple customer circuit. No exact R6 evaluation
board was substituted, and no physical hardware test is claimed.
