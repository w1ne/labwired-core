# Required Documents — STM32F072B-DISCO

Authoritative sources used by this example (retrieved 2026-10-23):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| RM0360 | STM32F0x1/2/8 reference manual | RCC §6 (CR/CFGR/APBxENR/CFGR2/CFGR3/CR2), GPIO §9, USART §27 (ISR/TDR), SPI §25, I2C §22 (legacy), FLASH §3, EXTI/SYSCFG, DMA §10 |
| DS9826 | STM32F072xB datasheet | 128 KiB flash / 16 KiB SRAM, 48 MHz maximum core clock, LQFP64 pin map, VIL/VIH thresholds |
| ST UM1690 (board manual) + Zephyr `stm32f072b_disco` board dts | the board | USART1 (PA9/PA10, AF1) as the simulator-captured console; ST-LINK/V2 is SWD-only, so physical serial needs an external USB-UART adapter; LD3 PC6, LD4 PC8 (orange), LD5 PC9, LD6 PC7 (active high); user button B1 on PA0 (active low) |
| STM32F0 CMSIS header (`stm32f072xb.h`) | the IRQ/vector map | USART1 IRQ 27, USART2 28, USART3/4 29, SPI1 25, SPI2 26, I2C1 23, I2C2 24, TIM1 13, TIM2 15, TIM3 16, TIM14 19, TIM15 20, TIM16 21, TIM17 22 |

ADC1 follows the RM0360 F0 `CFGR1`/`CHSELR` v2 register map; DS9826 Table 14
supplies the external ADC pad routes. The co-simulation test exercises NTC,
diode, and BJT conversions through that path (model coverage, not a silicon
capture). USB FS, TSC and CRS are declared stubs; the 4-direction joystick is
not modelled.
