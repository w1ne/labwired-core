# Required Documents — STM32F072B-DISCO

Authoritative sources used by this example (retrieved 2026-10-23):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| RM0360 | STM32F0x1/2/8 reference manual | RCC §6 (CR/CFGR/APBxENR/CFGR2/CFGR3/CR2), GPIO §9, USART §27 (ISR/TDR), SPI §25, I2C §22 (legacy), FLASH §3, EXTI/SYSCFG, DMA §10 |
| DS9826 | STM32F072xB datasheet | 128 KiB flash / 16 KiB SRAM, 48 MHz maximum core clock, LQFP64 pin map, VIL/VIH thresholds |
| ST UM1694 (board manual) + Zephyr `stm32f072b_disco` board dts | the board | USART1 (PA9/PA10, AF1) as the ST-LINK VCP console; LD3 PC6, LD4 PC8, LD5 PC9, LD6 PC7 (active high); user button B1 on PA0 (active low) |
| STM32F0 CMSIS header (`stm32f072xb.h`) | the IRQ/vector map | USART1 IRQ 27, USART2 28, USART3/4 29, SPI1 25, SPI2 26, I2C1 23, I2C2 24, TIM1 13, TIM2 15, TIM3 16, TIM14 19, TIM15 20, TIM16 21, TIM17 22 |

Open items: the ADC is modelled with the `stm32l4`-family profile (the
CFGR1/CFGR2 "v2" block) and is not exercised by the smoke; USB FS, TSC and CRS
are declared stubs; the 4-direction joystick is not modelled.
