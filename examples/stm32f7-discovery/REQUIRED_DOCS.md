# Required Source Documents (STM32F7 Discovery)

## MCU Datasheet / Reference Manual (authoritative)

1. STM32F746xx datasheet **DS10916** (memory map, peripheral bases):
   https://www.st.com/resource/en/datasheet/stm32f746ng.pdf
2. STM32F75xxx and STM32F74xxx reference manual **RM0385** (RCC, GPIO, USART, IRQs):
   https://www.st.com/resource/en/reference_manual/rm0385-stm32f75xxx-and-stm32f74xxx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf
3. CMSIS device header **stm32f746xx.h** (`USART1_IRQn = 37`, GPIOI / USART1 / RCC bases).

## Board Pinout / BSP

1. STM32F7 Discovery user manual **UM1974** (LD1 = PI1; ST-LINK VCP = USART1 on PA9/PA10):
   https://www.st.com/resource/en/user_manual/um1974-discovery-kit-with-stm32f746ng-mcu-stmicroelectronics.pdf

## Address Cross-Check Only (not a source of truth)

1. CMSIS `stm32f746xx.h` may be used to **cross-check** peripheral base addresses against RM0385 / DS10916. Do not treat third-party simulator platform files as authoritative for LabWired models.
2. Do not promote `core/configs/chips/onboarding/stm32f746.yaml` — that catalog stub uses non-native type names and incorrect flash/RAM sizing.
