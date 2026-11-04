# Vendor status checked 2026-11-04

- ST's [NUCLEO-L496ZG-P product page](https://www.st.com/en/evaluation-tools/nucleo-l496zg-p.html)
  lists the exact `-P` board as **Active** and says it is in volume production.
  It identifies the target as STM32L496ZGTP and has an Order Direct link.
- ST's [eStore listing](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards-page7.html)
  showed `NUCLEO-L496ZG-P` as **Active**, **In Stock**, **$26.35 at quantity 2**,
  with **Add to Cart** enabled. The adjacent `NUCLEO-L496ZG` item is marked
  NRND, so this onboarding selects and names only the active `-P` variant.
- ST's [STM32L496ZG product page](https://www.st.com/en/microcontrollers-microprocessors/stm32l496zg.html)
  lists STM32L496ZGT6P as **Active** and in volume production. ST lists 1 MiB
  flash and 320 KiB SRAM for this L496 line.

ST [UM2179 Rev 10](https://www.st.com/resource/en/user_manual/um2179-stm32-nucleo144-boards-mb1312-stmicroelectronics.pdf)
identifies the NUCLEO-L496ZG-P target as STM32L496ZGT6PU and maps red user LED
LD3 to PB14; the LED is active-high. ST [RM0351](https://www.st.com/resource/en/reference_manual/dm00083560-stm32l47xxx-stm32l48xxx-stm32l49xxx-and-stm32l4axxx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf)
defines GPIOB at `0x48000400`, RCC at `0x40021000`, and RCC_AHB2ENR.GPIOBEN at
bit 1. The profile exposes the 320 KiB contiguous SRAM window at `0x20000000`;
it does not separately map SRAM2's `0x10000000` alias.

The LabWired Part Knowledge query returned `NOT_FOUND` for both
`NUCLEO-L496ZG-P` and `STM32L496ZGT6P`, and recorded the catalog gap. The
descriptor therefore relies on ST's exact board manual and reference manual;
it does not infer facts from a neighboring part.
