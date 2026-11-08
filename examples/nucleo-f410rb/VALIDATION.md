# NUCLEO-F410RB validation sources and scope

- ST's [NUCLEO-F410RB product page](https://www.st.com/en/evaluation-tools/nucleo-f410rb.html)
  identifies the board and its fitted STM32F410RB.
- ST's [UM1724 Nucleo-64 manual](https://www.st.com/resource/en/user_manual/dm00105823.pdf),
  Table 20, maps D13 to PA5 and USART2 TX/RX to PA2/PA3 on NUCLEO-F410RB.
- ST's [STM32F410RB datasheet](https://www.st.com/resource/en/datasheet/stm32f410rb.pdf)
  and [RM0401 reference manual](https://www.st.com/resource/en/reference_manual/dm00180366-stm32f410-advanced-armbased-32bit-mcus-stmicroelectronics.pdf)
  support the narrow MCU profile.

The smoke exercises the RCC clock gates, USART2 transmit path, and GPIOA/PA5
output. Other chip peripherals and the ST-LINK USB bridge are omitted, so the
profile does not claim they work in the simulator.
