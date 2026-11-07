# NUCLEO-F446RE validation record

## Live vendor status checked 2026-11-03

- ST product page: <https://www.st.com/en/evaluation-tools/nucleo-f446re.html>
  reports **Active** and “Product is in volume production.”
- ST eStore: <https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-f446re.html>
  reports **In Stock**, $18.66 at quantity 2, with **Add to Cart**.
- MCU product page: <https://www.st.com/en/microcontrollers-microprocessors/stm32f446re.html>
  reports **Active** and identifies the STM32F446RE family as a 180 MHz Cortex-M4
  with up to 512 KiB flash and 128 KiB SRAM.

These are vendor pages, checked live before this descriptor was added; no
Wayback or archived status was used.

## Exercised scope

| Path | Evidence |
|---|---|
| Cortex-M4 reset vector and instruction execution | `evidence/logs/run_smoke.log` |
| USART2 TX marker `NUCLEO_F446RE_OK` | `evidence/logs/run_smoke.log`, `evidence/simulation-output/uart.log` |
| LD2 output on PA5 | `evidence/logs/run_smoke.log`, `evidence/simulation-output/result.json` |

The chip profile contains only RCC, GPIOA/GPIOC, SysTick, and USART2. It declares
no zero-answering stubs. The USER button mapping is present, but button input is
not part of the firmware smoke. SPI, I2C, ADC, timers, USB, DMA, and other F446
blocks are omitted and unsupported by this profile.

Reproduce with the command in `README.md`. The hosted board catalog does not yet
have a NUCLEO-F446RE entry, so this run is recorded as a local simulator smoke;
it is not a LabWired oracle `model_verified` result or hardware confirmation.
