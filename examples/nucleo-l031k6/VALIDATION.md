# Validation — NUCLEO-L031K6

The committed local smoke builds Cortex-M0+ firmware, selects the L0 HSI16
clock, enables GPIOB, and requires at least 16 transitions on PB3. The exact
board is not a target in the hosted `labwired_verify` catalog, so this report
makes no hosted model-verification claim. No physical board was tested.

| Capability | Evidence / limit |
| :--- | :--- |
| STM32L031K6T6 reset and firmware entry | Captured by the committed local smoke |
| STM32L0 RCC HSI16 selection and GPIOB clock gate | Firmware writes and polls the modeled RCC fields |
| NUCLEO-L031K6 LD3 / Arduino D13 on PB3 | Smoke requires at least 16 GPIO edges on `gpiob:3` |
| ST-LINK VCP, USB, ADC, I2C, SPI, timers, and other MCU peripherals | Omitted from the chip profile and not claimed |
| Zero-answering peripheral stubs | None declared |
| Physical STM32L031 silicon behavior | Not checked on a physical board |

Board wiring is documented in ST's [UM1956 Rev 6](https://www.st.com/resource/en/user_manual/um1956-stm32-nucleo32-boards-mb1180-stmicroelectronics.pdf),
§6.5 and Table 14. The local simulator output and generated metrics are under
[`evidence/`](evidence/).
