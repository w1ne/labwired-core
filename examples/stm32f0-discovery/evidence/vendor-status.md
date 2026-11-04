# Vendor status checked 2026-11-04

- ST's [STM32F0DISCOVERY product page](https://www.st.com/en/evaluation-tools/stm32f0discovery.html)
  lists the kit as **Active** and says it is in volume production. It identifies
  the target MCU as STM32F051R8T6.
- ST's [eStore listing](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-discovery-kits/stm32f0discovery.html)
  showed the kit as **Active**, **In Stock**, **$32.73 at quantity 2**, with
  **Add to Cart** enabled.
- ST's [STM32F051R8 product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f051r8.html)
  lists the MCU as **Active** and in volume production. ST's eStore product
  family page listed active STM32F051R8 orderable variants.

ST's [UM1525 board manual](https://www.st.com/resource/en/user_manual/um1525-stm32f0discovery-discovery-kit-for-stm32-f0-microcontrollers-stmicroelectronics.pdf)
maps green user LED LD3 to PC9 and identifies the board's STM32F051R8T6.
The [RM0091 reference manual](https://www.st.com/resource/en/reference_manual/rm0091-stm32f0x1stm32f0x2stm32f0x8-advanced-armbased-32bit-mcus-stmicroelectronics.pdf)
defines GPIOC at `0x48000800`, RCC at `0x40021000`, RCC AHBENR at offset
`0x14`, and GPIOC's clock-enable bit 19.

The LabWired Part Knowledge query for the exact `STM32F051R8` returned
`NOT_FOUND` in curated, vendor-SVD, and datasheet sources. This onboarding
records that catalog gap; it does not treat a neighboring F0 part as evidence.
