# NUCLEO-F030R8 validation record

## Live vendor status checked 2026-11-03

- ST's [NUCLEO-F030R8 product page](https://www.st.com/en/evaluation-tools/nucleo-f030r8.html)
  lists the board as **Active** and says it is in volume production.
- ST's [eStore board listing](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-f030r8.html)
  showed **In Stock**, **$19.90** at quantity 2, and **Add to Cart**.
- ST's [STM32F030R8 product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f030r8.html)
  lists the MCU as **Active** and in volume production; it showed STM32F030R8T6
  available from four distributors, including DigiKey, Farnell, Newark and
  Anglia Live.

## Board wiring and smoke scope

The board uses an STM32F030R8T6 in LQFP64. ST's [UM1724 Nucleo-64 board
manual](https://www.st.com/resource/en/user_manual/um1724-stlinkv21-in-circuit-debuggerprogrammer-for-stm8-and-stm32-stmicroelectronics.pdf),
[MB1136 C04 schematic](https://www.st.com/resource/en/schematic_pack/mb1136-default-c04_schematic.pdf),
and [Nucleo-64 data brief](https://www.st.com/resource/en/data_brief/nucleo-f030r8.pdf)
document this board family. UM1724 maps B1 USER to PC13, LD2 to D13/PA5, and
the default USART2 PA2/PA3 connection to the ST-LINK VCP.
The [STM32F030x8 datasheet](https://www.st.com/resource/en/datasheet/stm32f030r8.pdf)
maps USART2_TX/RX to PA2/PA3 on AF6; the smoke firmware selects that alternate
function and exercises USART2 and PA5 (LD2). PC13 is declared for the B1
button, but firmware does not read it. The ST-LINK debugger and USB VCP bridge
are not simulated; UART output is captured directly from the MCU model, so the
local smoke does not independently verify the physical AF/pad/bridge route.

## Evidence

The committed `evidence/logs/run_smoke.log` and
`evidence/simulation-output/` record the local engine run. The chip profile is
intentionally limited to the real RCC, SysTick, GPIOA/C and USART2 models; all
other STM32F030 peripherals are omitted and unsupported.
