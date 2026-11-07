# Required Source Documents (Feather-STM32F405)

## Board wiring and current availability

1. Adafruit [Feather STM32F405 Express product page](https://www.adafruit.com/product/4382): current board listing and MCU/board features.
2. Adafruit [Feather STM32F405 pinout guide](https://learn.adafruit.com/adafruit-stm32f405-feather-express/pinouts): D13 is PC1; D1/TX and D0/RX are USART3 on PB10/PB11; integrated devices and board pin functions.
3. Adafruit [board schematic](https://cdn-shop.adafruit.com/product-files/4382/4382-schematic.pdf): board-level wiring.
4. ST [STM32F405RG product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f405rg.html): current MCU lifecycle and orderable package variants. The live check is recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## MCU Reference Manual + Datasheet

1. RM0090 — STM32F405/407/415/417 reference manual (shared F4 IP; the F405
   descriptor reuses the `stm32f4` / `stm32v2` profiles):
   https://www.st.com/resource/en/reference_manual/dm00031020.pdf
2. DS8626 — STM32F405RG datasheet (memory sizes, package):
   https://www.st.com/resource/en/datasheet/stm32f405rg.pdf

## MCU (CMSIS Device Header)

1. STM32F405 device header (memory map, base addresses, IRQs):
   https://github.com/STMicroelectronics/cmsis_device_f4/blob/master/Include/stm32f405xx.h

## DBGMCU Identity

1. RM0090 §38.6.1 — DBGMCU_IDCODE: DEV_ID = 0x413 for STM32F405/407
   (REV_ID = 0x1000 → idcode 0x10006413).
