# External components

The smoke uses the STM32F429ZIT6 and the board's green user LED LD3 on PG13.
RCC and GPIOG use the engine's real peripheral models. The LCD, SDRAM,
I3G4250D gyroscope, USB circuitry, buttons and ST-LINK are not modeled in this
profile. No zero-answering stub windows are declared.
