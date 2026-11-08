# NUCLEO-F439ZI validation record

## Live vendor status checked 2026-11-03

- ST's [NUCLEO-F439ZI product page](https://www.st.com/en/evaluation-tools/nucleo-f439zi.html)
  lists the board as **Active** and says it is in volume production.
- ST's [eStore listing](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-f439zi.html)
  showed **Active**, **In Stock**, **$34.13** at quantity 2, and an order option.
- ST's [STM32F439ZI product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f439zi.html)
  lists STM32F439ZIT6 as **Active** and in volume production. It states up to
  2 MiB Flash and 256+4 KiB SRAM including 64 KiB CCM; this profile maps the
  192 KiB main SRAM region and omits CCM and backup SRAM.

## Board wiring and smoke scope

ST's [UM1974 Nucleo-144 manual](https://www.st.com/resource/en/user_manual/dm00244518-stm32-nucleo144-boardsstmicroelectronics.pdf)
maps the blue user LED LD2 to PB7 and states that user LEDs turn on when their
I/O is high. The [STM32F439ZI datasheet](https://www.st.com/resource/en/datasheet/stm32f439zi.pdf)
and [RM0090 reference manual](https://www.st.com/resource/en/reference_manual/dm00031020.pdf)
are the sources for the F439 memory and GPIO/RCC register map. The descriptor
exposes only the RCC reset-clock window, GPIOB, SysTick, and PB7. Firmware
enables GPIOB through RCC_AHB1ENR and toggles PB7 with GPIOB_BSRR; all other
peripherals and board components are omitted.

## Evidence

The committed `evidence/logs/run_smoke.log` and
`evidence/simulation-output/` record the local deterministic engine run. The
acceptance checks require at least eight PB7 transitions before the 256-step
limit. The hosted Playground catalog does not list STM32F439/NUCLEO-F439ZI, so
this evidence is from the repository's exact local system descriptor; it is not
a hosted `labwired_verify` verdict or hardware capture.
