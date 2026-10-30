# Required Documents — NUCLEO-L552ZE-Q

Authoritative sources used by this example (retrieved 2026-10-29):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| UM2581 | STM32L5 Nucleo-144 board (MB1361) user manual | Board facts: LD1/LD2/LD3 (PC7/PB7/PA9), B1 (PC13), ST-LINK/V2-1 Virtual COM Port on LPUART1 (PG7/PG8), headers |
| DS12737 | STM32L552xx datasheet | Memory map (512 KiB flash @ 0x08000000, 192 KiB SRAM1 @ 0x20000000 + 64 KiB SRAM2 @ 0x20030000), 110 MHz max, LQFP144 pinout, I/O static characteristics |
| RM0438 | STM32L552xx/L562xx reference manual | RCC chapter (AHB1ENR/AHB2ENR/AHB3ENR/APB1ENR1/APB1ENR2/APB2ENR, MSI/PLL handshakes), PWR chapter (PWR_CR2.IOSV — VDDIO2 isolation for PG[15:2]), GPIO, LPUART/USART, SPI, I2C, timers, DMA. Cited by chapter/register name and cross-checked against CMSIS `stm32l552xx.h` |
| ST CMSIS `stm32l552xx.h` | Device header | RCC/PWR/FLASH/EXTI register offsets used by the `stm32l5` RCC profile |
| Zephyr `nucleo_l552ze_q` board dts | Board support documentation | Cross-check of the default pin mapping (LPUART1 PG7/PG8 AF8, LEDs PC7/PB7/PA9, button PC13) |

The chip descriptor (`configs/chips/stm32l552.yaml`) carries the same values
transcribed with its own citations; this package does not restate them.

Open item: the L5 non-secure FLASH controller, EXTI, ADC, FDCAN and USB are
out of scope for this L1 slice and declared stubs in the descriptor.
TrustZone/SAU/GTZC and MPU/PMSAv8 enforcement are engine gaps, not board
documentation gaps.
