# External components and omitted board features

The smoke uses the STM32L496ZGT6P's RCC and GPIOB models to exercise onboard
red user LED LD3 on PB14. It does not model the ST-LINK/V2-1, USB, user/reset
buttons, LD1/LD2/COM/PWR LEDs, SMPS behavior, or header connections. GPIO ports
other than GPIOB and all other MCU peripherals are omitted from this profile.
