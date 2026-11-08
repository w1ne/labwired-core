# NUCLEO-F072RB

This board system uses ST's STM32F072RBT6 and maps LD2 to PA5, B1 to PC13,
and the on-board ST-LINK V2-1 virtual COM port to USART2 on PA2/PA3. The
smoke boots a Cortex-M0 firmware image, sends `NUCLEO_F072RB_OK` over USART2,
and checks eight LD2 transitions on PA5.

## Reproduce

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-f072rb \
  --crate firmware-nucleo-f072rb-smoke \
  --target thumbv6m-none-eabi \
  --script examples/nucleo-f072rb/io-smoke.yaml \
  --system configs/systems/nucleo-f072rb.yaml \
  --out-dir examples/nucleo-f072rb/evidence
```

The committed `evidence/logs/run_smoke.log` records the test result and
`evidence/simulation-output/` contains the UART and GPIO observations.

## Live vendor status checked 2026-11-03

ST's [NUCLEO-F072RB product page](https://www.st.com/en/evaluation-tools/nucleo-f072rb.html)
states **Active** and “Product is in volume production.” The [ST eStore
listing](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-f072rb.html)
showed **In Stock**, **$20.62** at quantity 2, and **Add to Cart**. ST's
[STM32F072RB product page](https://www.st.com/en/microcontrollers-microprocessors/stm32f072rb.html)
also states **Active** and “Product is in volume production.”

## Model boundary

The board uses a narrow STM32F072 model containing only RCC, SysTick,
GPIOA/C, and USART2. The smoke exercises RCC clock gates, GPIOA/PA5 and
USART2. The USER button map is present but is not read by the firmware. USB,
touch sensing (TSC), clock recovery (CRS), and the remaining chip blocks are
omitted; accesses to them are unsupported. This is simulator evidence; there
is no NUCLEO-F072RB silicon capture in this evidence set. The simulator
captures USART2 output directly; it does not model the ST-LINK debugger or its
USB VCP bridge.
