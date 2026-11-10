# Required source documents — NUCLEO-F103RB

| Document | Source | Used for |
|---|---|---|
| STM32F103x8/xB datasheet, DS5319 Rev 20 | https://www.st.com/resource/en/datasheet/stm32f103rb.pdf | STM32F103RB flash/SRAM, Cortex-M3 maximum clock, LQFP64 pinout, GPIO/ADC geometry, I/O levels, and ordering scheme |
| STM32 Nucleo-64 boards user manual, UM1724 Rev 17 | https://www.st.com/resource/en/user_manual/DM00105823.pdf | NUCLEO-F103RB carrier identity; LD2 on PA5 active-high; B1 on PC13; USART2 PA2/PA3 and default ST-LINK VCP solder-bridge routing |
| STM32F1 reference manual, RM0008 | https://www.st.com/resource/en/reference_manual/cd00171190.pdf | F1 memory map and RCC, GPIO, USART, I2C, AFIO, and peripheral register layouts |
| ST CMSIS STM32F10x device headers | https://github.com/STMicroelectronics/cmsis_device_f1 | Peripheral base addresses and interrupt numbers used by the family model |

The exact chip used by the NUCLEO-F103RB is `STM32F103RB`; the board commonly
ships with STM32F103RBT6. The dedicated chip descriptor records the 128 KB
flash and 20 KB SRAM geometry and the LQFP64 bond-out.

The companion [BME280 profile](../nucleo-f103rb-bme280/README.md) wires its
breakout to default I2C1 PB6/SCL and PB7/SDA and uses address `0x77`. ST's
UM1724 pinout and the F1 reference manual identify the board pins and I2C1
register sequence; the Bosch [BME280 datasheet](https://www.bosch-sensortec.com/media/boschsensortec/downloads/datasheets/bst-bme280-ds001.pdf)
documents register `0xD0` as chip ID. The dated lifecycle/availability check
for this combination is in the companion example's
[`evidence/vendor-status.md`](../nucleo-f103rb-bme280/evidence/vendor-status.md).
