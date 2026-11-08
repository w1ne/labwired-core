# NUCLEO-F072RB validation record

## Vendor status checked live 2026-11-03

- ST's [board product page](https://www.st.com/en/evaluation-tools/nucleo-f072rb.html)
  lists NUCLEO-F072RB as **Active** and “Product is in volume production.”
- ST's [eStore listing](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-f072rb.html)
  showed **In Stock**, $20.62 at quantity 2, and **Add to Cart**.
- ST's [STM32F072RB product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f072rb.html)
  lists the MCU as **Active** and in volume production.

## Exercised paths

| Path | Evidence |
|---|---|
| Cortex-M0 reset and instruction execution | `evidence/logs/run_smoke.log` |
| USART2 marker `NUCLEO_F072RB_OK` | `evidence/simulation-output/uart.log` |
| LD2 GPIO output transitions on PA5 | `evidence/simulation-output/result.json` |

The board pin map follows ST's [UM1724 Nucleo-64 board manual](https://www.st.com/resource/en/user_manual/um1724-stlinkv21-in-circuit-debuggerprogrammer-for-stm8-and-stm32-stmicroelectronics.pdf)
and the NUCLEO-F072RB schematic linked from the board product page. Its narrow
chip descriptor contains RCC, SysTick, GPIOA/C, and USART2 real models only.
USB, TSC, CRS, and all other peripherals are omitted, so their accesses are
unsupported. This is a local engine smoke, not a LabWired oracle
`model_verified` result or hardware confirmation. USART2 output is captured
directly by the simulator; the ST-LINK debugger and USB VCP bridge are not
simulated.
