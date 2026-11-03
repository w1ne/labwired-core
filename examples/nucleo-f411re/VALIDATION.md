# NUCLEO-F411RE validation record

## Vendor status checked live 2026-11-03

- ST's [NUCLEO-F411RE board page](https://www.st.com/en/evaluation-tools/nucleo-f411re.html)
  lists the board as **Active** and “Product is in volume production.”
- The linked [ST eStore listing](https://estore.st.com/en/nucleo-f411re-cpn.html)
  shows **In stock**, **$18.53** for one unit, and an **Add to cart** control.
- ST's [STM32F411RE product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f411re.html)
  lists STM32F411RET6 as **Active** and “Product is in volume production.”

## Exercised paths

| Path | Evidence |
|---|---|
| Cortex-M4 reset and instruction execution | `evidence/logs/run_smoke.log` |
| USART2 marker `NUCLEO_F411RE_OK` | `evidence/simulation-output/uart.log` |
| LD2 output transitions on GPIOA/PA5 | `evidence/simulation-output/result.json` |

The board pin map is based on ST UM1724 and the NUCLEO-F411RE schematic linked
from the board product page. The narrow chip descriptor contains modeled RCC,
GPIOA and USART2 only; no zero-answering stubs are declared. Other
peripherals and board paths are unsupported by this system profile.

This is a local repository CLI smoke. The hosted LabWired Playground catalog
does not list NUCLEO-F411RE, so this is not a hosted `labwired_verify` verdict
or hardware confirmation. The simulator captures USART2 output directly; it
does not simulate the ST-LINK debugger or USB VCP bridge.
