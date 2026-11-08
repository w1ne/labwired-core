# NUCLEO-C051C8

The NUCLEO-C051C8 is an ST Nucleo-64 development board with an STM32C051C8.
The initial LabWired system profile covers a deliberately small board path:

- STM32C051C8 reset-clock boot
- RCC IOPENR GPIOA clock enable
- GPIOA output driving the active-high LD4 user LED on PA5

The smoke firmware toggles LD4 and asserts at least 16 observed GPIO edges.
The profile omits the board buttons, VCP/UART, USB, and expansion connectors;
those paths are not claimed as modeled. The engine idealizes power rails, so
power-on and brown-out behavior are outside this evidence.

Vendor status and committed local smoke evidence: [the board onboarding
README](../../examples/nucleo-c051c8/README.md).

Board wiring follows [ST UM2953 Rev 2 for MB1717](https://www.st.com/resource/en/user_manual/um2953-stm32-nucleo64-board-mb1717-stmicroelectronics.pdf)
and the [NUCLEO-C051C8 schematic](https://www.st.com/resource/en/schematic_pack/mb1717-c051c8-c02-schematic.pdf).
MCU memory sizing follows the [STM32C051x6/x8 datasheet](https://www.st.com/resource/en/datasheet/stm32c051c8.pdf).
