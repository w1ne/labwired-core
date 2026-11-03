# NUCLEO-G070RB

This ST Nucleo-64 board uses the STM32G070RBT6. The board manifest maps LD4
to PA5, B1 to PC13, and the ST-LINK virtual COM port to USART2 on PA2/PA3.
The chip descriptor is an intentionally narrow smoke profile, not a claim
that every G070 peripheral is modeled.

The smoke reuses `firmware-stm32g0-demo`. It enables the G0 GPIO and USART2
clock gates, prints `OK` through the VCP, and toggles LD4. The test asserts
both the UART marker and at least eight PA5 edges.

## Reproduce

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-g070rb \
  --crate firmware-stm32g0-demo \
  --target thumbv6m-none-eabi \
  --script examples/nucleo-g070rb/io-smoke.yaml \
  --system configs/systems/nucleo-g070rb.yaml \
  --out-dir examples/nucleo-g070rb/evidence
```

The committed `evidence/logs/run_smoke.log` and
`evidence/onboarding-metrics.json` record the run.

## Model boundary

The profile contains RCC, SysTick, GPIOA/C, and USART2. The smoke exercises
GPIOA/PA5 and USART2. Other G070 peripherals, USB, the ST-LINK debug
transport, PLL operation, and unlisted GPIO pins are not claimed. This is
simulator evidence only; no silicon capture is available for this board. No
zero-answering stub peripherals are declared; accesses outside the profile
remain unsupported.

## Vendor status checked 2026-11-03

ST's live [NUCLEO-G070RB product page](https://www.st.com/en/evaluation-tools/nucleo-g070rb.html)
states **Active** and “Product is in volume production.” The [ST eStore
listing](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-g070rb.html)
showed **In Stock**, **$19.90**, and **Add to Cart**. ST's live
[STM32G070RB product page](https://www.st.com/en/microcontrollers-microprocessors/stm32g070rb.html)
also states **Active** and “Product is in volume production”; the eStore lists
the STM32G070RBT6TR tape-and-reel variant In Stock.

Board wiring follows ST's [UM2324](https://www.st.com/resource/en/user_manual/um2324-stm32-nucleo64-boards-mb1360-stmicroelectronics.pdf)
and [MB1360-G070RB schematic](https://www.st.com/resource/en/schematic_pack/mb1360-g070rb-c02_schematic.pdf).
The MCU memory and peripheral map follow ST's
[STM32G070RB datasheet](https://www.st.com/en/microcontrollers-microprocessors/stm32g070rb.html)
and [RM0444](https://www.st.com/resource/en/reference_manual/rm0444-stm32g0x1-advanced-armbased-32bit-mcus-stmicroelectronics.pdf).
