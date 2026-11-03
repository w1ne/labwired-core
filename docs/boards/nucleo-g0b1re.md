# NUCLEO-G0B1RE

The NUCLEO-G0B1RE is an ST Nucleo-64 development board built around the
STM32G0B1RET6. Its system profile is
[`configs/systems/nucleo-g0b1re.yaml`](../../configs/systems/nucleo-g0b1re.yaml)
and uses the existing narrow MCU model in
[`configs/chips/stm32g0b1re.yaml`](../../configs/chips/stm32g0b1re.yaml).

## Vendor status

Checked 2026-11-03 against ST's live pages:

- ST's [NUCLEO-G0B1RE product page](https://www.st.com/en/evaluation-tools/nucleo-g0b1re.html)
  states **Active** and “Product is in volume production.”
- ST's [eStore listing](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-g0b1re.html)
  showed **In Stock** and **Add to Cart**.
- ST's [STM32G0B1RE product page](https://www.st.com/en/microcontrollers-microprocessors/stm32g0b1re.html)
  states **Active** and “Product is in volume production.”

## Board wiring

| Board signal | MCU connection | Model use |
|---|---|---|
| LD4 user LED | PA5, active high | GPIO smoke toggles it |
| B1 user button | PC13, active low | Declared as board input |
| ST-LINK virtual COM port | USART2 TX PA2 / RX PA3 | UART smoke output |

The board wiring follows ST's [UM2324 Nucleo-64 user manual](https://www.st.com/resource/en/user_manual/dm00452640-stm32-nucleo64-boards-with-stm32g07xrb-mcus-stmicroelectronics.pdf)
and [MB1360-G0B1RE-C02 schematic](https://www.st.com/resource/en/schematic_pack/mb1360-g0b1re-c02_schematic.pdf).
The STM32G0 register addresses and RCC layout follow ST's
[RM0444](https://www.st.com/resource/en/reference_manual/rm0444-stm32g0x1-advanced-armbased-32bit-mcus-stmicroelectronics.pdf)
and [STM32G0B1RE datasheet](https://www.st.com/resource/en/datasheet/stm32g0b1re.pdf).

## Smoke and evidence

[`examples/nucleo-g0b1re/io-smoke.yaml`](../../examples/nucleo-g0b1re/io-smoke.yaml)
builds `firmware-stm32g0-demo` for Cortex-M0+. It asserts `OK` on USART2 and
at least eight PA5 edges while the firmware toggles LD4. The committed run is
in [`examples/nucleo-g0b1re/evidence`](../../examples/nucleo-g0b1re/evidence/).
The same smoke is a cell in the nightly coverage-matrix workflow.

## Model limits

The STM32G0B1RE profile declares RCC, SysTick, GPIOA/B/C, USART1/2, TIM2,
EXTI, NVIC, and SYSCFG. SYSCFG remains a zero-answering stub. The board smoke
exercises GPIOA/PA5 and USART2 only. I2C, SPI, ADC, DMA, FDCAN, USB, PLL
operation, and GPIO ports D/F are omitted or unsupported by this profile.
No silicon capture or executing-fidelity differential is available, so this
is simulator evidence and does not claim hardware confirmation.
