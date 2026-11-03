# NUCLEO-F303RE validation record

## Live vendor status checked 2026-11-03

- ST's [NUCLEO-F303RE product page](https://www.st.com/en/evaluation-tools/nucleo-f303re.html)
  lists the board as **Active** and says it is in volume production.
- ST's [eStore board listing](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-f303re.html)
  showed **In Stock**, **$19.90** at quantity 2, and **Add to Cart**.
- ST's [STM32F303RE product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f303re.html)
  lists the MCU as **Active** and in volume production. The page includes
  STM32F303RET6 and an ST direct-buy/free-sample link; the product family is
  listed with 512-KB Flash and 80-KB SRAM, of which this profile maps the
  64-KB main SRAM region and leaves the separate CCM region omitted.
- ST's [STM32F303RE eStore listing](https://estore.st.com/en/products/microcontrollers-microprocessors/stm32-32-bit-arm-cortex-mcus/stm32-mixed-signal-mcus/stm32f3-series/stm32f303/stm32f303re.html)
  showed STM32F303RET6 **Active** and **In Stock** with **Add to Cart**.

## Board wiring and smoke scope

ST's [UM1724 Nucleo-64 board manual](https://www.st.com/resource/en/user_manual/dm00105823.pdf)
maps NUCLEO-F303RE D13 to PA5, connects LD2 to D13, and documents the default
ST-LINK USART2 path on PA2/PA3. ST's [DS10362 datasheet](https://www.st.com/resource/en/datasheet/stm32f303ze.pdf)
maps PA2/PA3 to USART2 TX/RX on AF7. The [RM0316 reference manual](https://www.st.com/resource/en/reference_manual/dm00043574.pdf)
places RCC_AHBENR at offset 0x14 (GPIOAEN bit 17) and RCC_APB1ENR at offset
0x1C (USART2EN bit 17). DS10362 describes 512-KB Flash, 64-KB main SRAM, and
16-KB CCM SRAM; the descriptor maps Flash and the main SRAM only. It exposes
only USART2, GPIOA, SysTick, and the RCC window through APB1ENR. The smoke
exercises only the AHBENR/APB1ENR clock-gate effects; other register behavior
inside that window is not validated. RCC registers at offsets 0x20 and above,
the F303 mixed-signal blocks, CCM SRAM, ST-LINK probe, and USB VCP bridge are
omitted.

## Evidence

The committed `evidence/logs/run_smoke.log` and
`evidence/simulation-output/` record the local engine run. The acceptance
checks require the USART2 marker, at least eight LD2 GPIO edges, and the
expected max-step stop condition.
