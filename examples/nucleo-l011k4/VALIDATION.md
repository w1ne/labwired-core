# Validation — NUCLEO-L011K4

This smoke exercises the NUCLEO-L011K4's USART2 TX and user LED using the
repository's local simulator and the focused descriptor. It does not assert
the physical ST-LINK USB bridge or the electrical AF route.

The hosted `labwired_verify` catalog has no NUCLEO-L011K4 target, so this
package records the local smoke only and makes no hosted verification claim.

| Capability | Evidence / limit |
| :--- | :--- |
| Cortex-M0+ entry with 16 KiB flash and 2 KiB SRAM | Captured by the committed local smoke |
| STM32L0 RCC HSI16 selection and GPIOA/B + USART2 clock enables | Firmware writes and polls these registers; the local model decodes the L0 register map |
| D13 / PB3 active-high user LED | Smoke requires at least 16 GPIO edges on `gpiob:3` |
| USART2 TX on PA2 / AF4 | Smoke requires the banner on the modeled USART2 console; the external ST-LINK/V2-1 USB bridge is not modeled |
| USART2 RX on PA15 / AF4 | Pin function is documented, but no external transmitter or RX behavior is exercised by this smoke |
| ADC, I2C, SPI, timers, DMA, EEPROM, LPUART, and other MCU/board functions | Not declared by this profile and not claimed |
| Physical STM32L011 silicon behavior | Not checked on a physical board |

The pin map is based on ST's [UM1956 Rev 6](https://www.st.com/resource/en/user_manual/um1956-stm32-nucleo32-boards-mb1180-stmicroelectronics.pdf)
and [STM32L011K4 datasheet](https://www.st.com/resource/en/datasheet/stm32l011k3.pdf).
The committed simulator logs and generated metrics are under
[`evidence/`](evidence/).
