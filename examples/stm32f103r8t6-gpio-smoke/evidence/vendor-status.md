# STM32F103R8 live vendor status

Checked on **2026-11-06 UTC**.

- ST's [STM32F103R8 product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f103r8.html)
  says **Active** and **“Product is in volume production.”** It lists the
  `STM32F103R8T6TR` orderable part and links directly to the ST eStore.
- ST's [STM32F103R8T6TR eStore page](https://estore.st.com/en/stm32f103r8t6tr-cpn.html)
  returned the exact part as **Active** and **In stock** (the live page's
  product offer reported `schema.org/InStock`). This is the orderable LQFP64
  R8T6 part represented by the smoke profile.
- ST's [STM32F103R8 datasheet](https://www.st.com/resource/en/datasheet/stm32f103r8.pdf)
  is the source referenced by the exact package/memory descriptor.

This is a bare-part smoke fixture with an external LED on PC13, not a named
development board.
