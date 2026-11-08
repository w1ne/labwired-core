# External components and omitted board features

The smoke uses only the STM32F051R8T6's RCC and GPIOC models to exercise
onboard LED LD3 on PC9. It does not model the ST-LINK/V2, USB connection,
user/reset buttons, LD2 communication LED, LD4 on PC8, or the additional
header/prototyping board. Other STM32F051 peripherals and GPIO ports are
omitted from this board profile.
