# External components and omitted board features

The smoke uses the modeled STM32H7 RCC and GPIOB path for green user LED LD1
on PB0. The profile contains no external devices and no zero-answering stubs.
It omits the STM32H755's Cortex-M4 core, the other onboard LEDs and button,
ST-LINK, Ethernet, USB, SMPS behavior, and all other MCU peripherals.
