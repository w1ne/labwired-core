# NUCLEO-H7A3ZI-Q live vendor status

Checked live on 2026-11-04 against ST's eStore and board documentation:

- ST's [NUCLEO-H7A3ZI-Q product page](https://www.st.com/en/evaluation-tools/nucleo-h7a3zi-q.html)
  marked the board **Active** and **in volume production**. Its [eStore listing](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-h7a3zi-q.html)
  marked it **In Stock** and offered **Add to Cart** at **$40.56** for the
  displayed two-unit tier.
- ST's [STM32H7A3ZI product page](https://www.st.com/en/microcontrollers-microprocessors/stm32h7a3zi.html)
  marked the device family and exact **STM32H7A3ZIT6Q** order code **Active**
  and in volume production. ST's [eStore listing](https://estore.st.com/en/products/microcontrollers-microprocessors/stm32-32-bit-arm-174-cortex-174-mcus/stm32-high-performance-mcus/stm32h7-series/stm32h7a3-7b3/stm32h7a3zi.html)
  showed the exact fitted suffix **Out of Stock** with a stock-notification
  option; related **STM32H7A3ZIT6** was **In Stock** with Add to Cart. The
  board listing itself was orderable at this check.
- ST's [NUCLEO-144 data brief](https://www.st.com/resource/en/data_brief/nucleo-h7a3zi-q.pdf)
  maps NUCLEO-H7A3ZI-Q to board reference MB1363 and target
  STM32H7A3ZIT6Q.
- ST's [UM2408 Rev 6 user manual](https://www.st.com/resource/en/user_manual/um2408-stm32h7-nucleo144-boards-mb1363-stmicroelectronics.pdf)
  identifies LD1 on PB0 for SB54 ON / SB65 OFF and states the user LEDs turn on
  when their I/O is high.
- ST's [DS13195 Rev 8 datasheet](https://www.st.com/resource/en/datasheet/stm32h7a3zi.pdf)
  lists the H7A3ZIT6Q as a Cortex-M7 device with 2 MiB flash and 128 KiB DTCM.
- ST's [RM0455 Rev 12 reference manual](https://www.st.com/resource/en/reference_manual/dm00463927-stm32h7a3-b3-and-stm32h7b0-value-line-advanced-arm-based-32-bit-mcus-stmicroelectronics.pdf)
  documents the H7A3 RCC/GPIO peripheral map and the DTCM base address
  `0x20000000`.

The board was buyable from ST on the check date. The exact fitted MCU suffix is
active but was not separately in stock; current stock can change.
