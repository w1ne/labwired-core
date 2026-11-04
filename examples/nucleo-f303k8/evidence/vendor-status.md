# Live vendor status: NUCLEO-F303K8

Checked 2026-11-04 against ST's live product and eStore pages.

- [ST NUCLEO-F303K8 product page](https://www.st.com/en/evaluation-tools/nucleo-f303k8.html)
  lists the board as **Active** and says it is in volume production. Its live
  sample-and-buy section showed four distributors with stock, with inventory
  reported on 2026-10-31: Farnell 1252, Anglia 275, Mouser 1177, and DigiKey
  1077; the page also offers direct ordering and free samples.
- [ST eStore NUCLEO-F303K8 listing](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-f303k8.html)
  showed **In Stock** and **Add to Cart**, at $20.13 for quantity 2.
- [ST eStore STM32F303K8T6 listing](https://estore.st.com/en/products/microcontrollers-microprocessors/stm32-32-bit-arm-cortex-mcus/stm32-mainstream-mcus/stm32f3-series/stm32f303/stm32f303k8.html)
  lists the exact board MCU as **Active**, **In Stock**, and **Add to Cart**;
  the listing showed $4.14 at quantity 500.
- [ST STM32F303K8 datasheet](https://www.st.com/resource/en/datasheet/stm32f303k8.pdf)
  identifies the F303K8 as a Cortex-M4 with FPU and documents the x6/x8 memory
  layout: 64 KiB Flash, 12 KiB main SRAM, and a separate 4 KiB CCM SRAM bank.
- [ST UM1956, Nucleo-32 board manual](https://www.st.com/resource/en/user_manual/dm00231744-stm32-nucleo32-boards-mb1180-stmicroelectronics.pdf)
  documents the NUCLEO-F303K8 VCP on PA2 TX / PA15 RX by default and LD3
  connected to D13/PB3. The smoke uses only TX and the LED.
