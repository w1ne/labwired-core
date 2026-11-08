# NUCLEO-G070RB

The NUCLEO-G070RB is an ST Nucleo-64 board built around the STM32G070RBT6.
Its system manifest uses a deliberately narrow STM32G070RB profile for boot,
GPIO, and UART smoke coverage.

## Vendor status

Checked 2026-11-03 against ST's live pages:

- ST's [NUCLEO-G070RB product page](https://www.st.com/en/evaluation-tools/nucleo-g070rb.html)
  states **Active** and “Product is in volume production.”
- ST's [eStore listing](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-g070rb.html)
  showed **In Stock**, **$19.90**, and **Add to Cart**.
- ST's [STM32G070RB product page](https://www.st.com/en/microcontrollers-microprocessors/stm32g070rb.html)
  states **Active** and “Product is in volume production.” The eStore lists
  STM32G070RBT6TR as **In Stock**.

## Board wiring

| Board signal | MCU connection | Model use |
|---|---|---|
| LD4 user LED | PA5, active high | GPIO smoke toggles it |
| B1 user button | PC13, active low | Declared as board input |
| ST-LINK virtual COM port | USART2 TX PA2 / RX PA3 | UART smoke output |

The board pinout follows ST's [UM2324](https://www.st.com/resource/en/user_manual/um2324-stm32-nucleo64-boards-mb1360-stmicroelectronics.pdf)
and [MB1360-G070RB schematic](https://www.st.com/resource/en/schematic_pack/mb1360-g070rb-c02_schematic.pdf).
The chip's memory and peripheral addresses follow ST's
[STM32G070RB datasheet](https://www.st.com/en/microcontrollers-microprocessors/stm32g070rb.html)
and [RM0444](https://www.st.com/resource/en/reference_manual/rm0444-stm32g0x1-advanced-armbased-32bit-mcus-stmicroelectronics.pdf).

## Smoke and evidence

[`examples/nucleo-g070rb/io-smoke.yaml`](../../examples/nucleo-g070rb/io-smoke.yaml)
builds and runs `firmware-stm32g0-demo` for Cortex-M0+. It asserts the `OK`
UART marker and at least eight PA5 edges. The committed run is in
[`examples/nucleo-g070rb/evidence`](../../examples/nucleo-g070rb/evidence/).

## Model limits

The chip profile declares only RCC, SysTick, GPIOA/C, and USART2. The smoke
exercises GPIOA/PA5 and USART2. Other G070 peripherals, USB, ST-LINK debug
transport, PLL operation, and unlisted GPIO pins are omitted. No silicon
capture or executing-fidelity differential is available, so this is
simulator evidence and does not claim hardware confirmation. No
zero-answering stub peripherals are declared in this profile; accesses
outside these windows are unsupported.
