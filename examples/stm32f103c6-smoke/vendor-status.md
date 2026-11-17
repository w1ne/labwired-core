# STM32F103C6T6A live vendor status

Checked live on **2026-11-17 UTC** for the exact order code used by this
example, `STM32F103C6T6A`.

- ST's [STM32F103C6 product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f103c6.html)
  states **Active** and **Product is in volume production**. The C6 family page
  lists the C6T6A order code.
- ST's [official eStore listing for STM32F103C6T6A](https://estore.st.com/en/stm32f103c6t6a-cpn.html)
  states **Active**, **In stock**, and offers **Add to cart** for quantity 1.
  The listing identifies the package as LQFP48 and the MCU as a 32 KB Flash
  STM32F103C6.
- ST's [DS5936 low-density datasheet](https://www.st.com/resource/en/datasheet/stm32f103c6.pdf)
  identifies the x6 variant as 32 KB Flash / 10 KB SRAM and defines the package
  pin map used by the PA2/PA3/PA5 example.
- ST's [RM0008 reference manual](https://www.st.com/resource/en/reference_manual/rm0008-stm32f103xx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf)
  §31.6.1 specifies debug-port low-density `REV_ID=0x1000` and `DEV_ID=0x412`.
- ST's [ES0348 errata sheet](https://www.st.com/resource/en/errata_sheet/es0348-stm32f101x46-stm32f102x46-stm32f103x46-device-errata-stmicroelectronics.pdf)
  §2.2.2 says `DBGMCU_IDCODE` is accessible only in debug mode and reads zero
  from user firmware. The C6 configuration models that CPU-visible zero; the
  smoke does not attempt to read the debug-only identity register.

This is a bare MCU part profile with a simple customer circuit. No exact C6
development board was substituted, and no physical hardware test is claimed.
