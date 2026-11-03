# NUCLEO-F091RC validation record

## Board mapping and model scope

The MCU profile targets the STM32F091RCT6 installed on this Nucleo-64 board.
ST's STM32F091xB/xC datasheet gives this part a Cortex-M0 core, 256 KiB Flash,
32 KiB SRAM, an 8 MHz internal reset clock, and USART2_TX/RX on PA2/PA3 using
AF1. ST UM1724 maps the board's LD2 LED to PA5 and the board VCP to USART2 on
PA2/PA3. The profile models only RCC, SysTick, GPIOA and USART2; it does not
declare the other F091 peripherals, the B1 user button, shields or ST-LINK.

The smoke asserts the USART2 marker and at least eight PA5 output edges in the
local repository simulator. USART output is captured at the MCU model, so this
does not verify the physical pad routing, ST-LINK bridge, or board hardware.

## Evidence

`evidence/logs/run_smoke.log` contains the local engine smoke result.
`evidence/simulation-output/` contains the UART output and simulator result.
The hosted LabWired oracle catalog currently has no F091 board target, so this
record does not claim a hosted `model_verified` verdict or silicon confirmation.

The MCU and board details are sourced from ST's
[STM32F091RC datasheet](https://www.st.com/resource/en/datasheet/stm32f091rc.pdf),
[RM0091 reference manual](https://www.st.com/resource/en/reference_manual/dm00031936-stm32f0x1stm32f0x2stm32f0x8-advanced-armbased-32bit-mcus-stmicroelectronics.pdf),
and [UM1724 Nucleo-64 manual](https://www.st.com/resource/en/user_manual/um1724-stlinkv21-in-circuit-debuggerprogrammer-for-stm8-and-stm32-stmicroelectronics.pdf).

See [`evidence/vendor-status.md`](evidence/vendor-status.md) for the live ST
status and ordering check made for this onboarding round.
