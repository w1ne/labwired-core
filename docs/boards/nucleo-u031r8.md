# NUCLEO-U031R8

The ST NUCLEO-U031R8 is a Nucleo-64 board based on the STM32U031R8T6. The
narrow LabWired system profile is
[`configs/systems/nucleo-u031r8.yaml`](../../configs/systems/nucleo-u031r8.yaml);
its chip slice is
[`configs/chips/stm32u031r8-nucleo-smoke.yaml`](../../configs/chips/stm32u031r8-nucleo-smoke.yaml).

## Board path

| Board signal | MCU connection | Modeled use |
| --- | --- | --- |
| Green user LED LD4 / Arduino D13 | PA5, active high | GPIO smoke toggles it |

The mapping follows ST's [UM3256 board manual](https://www.st.com/resource/en/user_manual/um3256-stm32-nucleo64-boards-mb1932-stmicroelectronics.pdf)
and the [MB1932-U031R8-B01 schematic](https://www.st.com/resource/en/schematic_pack/mb1932-u031r8-b01-schematic.pdf).
Zephyr's [NUCLEO-U031R8 board DTS](https://github.com/zephyrproject-rtos/zephyr/blob/main/boards/st/nucleo_u031r8/nucleo_u031r8.dts)
also identifies LD4 as active-high on PA5.

## Vendor status

Checked live on 2026-11-04 UTC:

- ST's [board product page](https://www.st.com/en/evaluation-tools/nucleo-u031r8.html)
  states **Active** and “Product is in volume production.”
- ST's [eStore listing](https://estore.st.com/en/nucleo-u031r8-cpn.html)
  showed **In Stock** and **Add to cart**.
- ST's [STM32U031R8 product page](https://www.st.com/en/microcontrollers-microprocessors/stm32u031r8.html)
  states **Active** and “Product is in volume production.”

## Smoke and limits

[`examples/nucleo-u031r8/onboarding-smoke.yaml`](../../examples/nucleo-u031r8/onboarding-smoke.yaml)
boots bare-metal Cortex-M0+ firmware and asserts at least 16 output edges on
PA5. The profile contains only the modeled U0 RCC GPIOA clock gate and GPIOA;
it has no zero-answering stubs. USART2/VCP, USB, ST-LINK, buttons, timers, and
remaining STM32U031 behavior are intentionally unclaimed. The smoke is local
simulator evidence, not physical-board confirmation or a hosted
`labwired_verify` verdict.
