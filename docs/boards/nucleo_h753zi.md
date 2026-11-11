# ST NUCLEO-H753ZI

The **NUCLEO-H753ZI** is an ST Nucleo-144 board with the STM32H753ZIT6
Cortex-M7 MCU. ST currently lists the board as active and in volume production.
Its eStore listed it in stock at $40.26 for two units on 2026-11-11. See the
[live vendor record](../../examples/nucleo-h753zi/vendor-status.md).

## Demand position

The demand list ranks other STM32 models fourth at **13%** of reported MCU use.
Nucleo/Discovery kits appear in **23%** of projects.

## Working profile

The exact chip descriptor is
[`configs/chips/stm32h753zi.yaml`](../../configs/chips/stm32h753zi.yaml), and
the board wiring is in
[`configs/systems/nucleo-h753zi.yaml`](../../configs/systems/nucleo-h753zi.yaml).
The profile uses the engine's H7 RCC, GPIOB/C/D, USART3, and SysTick models.
It binds the default green LD1 to PB0 and B1 to PC13.

The firmware smoke prints `H753_USART3_OK` through the default USART3 VCP on
PD8/PD9 and checks at least eight LD1 output edges on PB0. Build and run it
with the [quick start](../../examples/nucleo-h753zi/README.md). The run passed
3/3 checks; logs and machine-readable output are committed under
[`examples/nucleo-h753zi/evidence/`](../../examples/nucleo-h753zi/evidence/).

The model captures the USART3 register output and GPIO state. It does not
emulate the ST-LINK VCP bridge or prove pad voltage. Other LEDs, the full
Nucleo-144 header, Ethernet, USB, and other H753 peripheral blocks are not
modeled in this board profile. This is simulator evidence; no physical-board
capture is claimed.
