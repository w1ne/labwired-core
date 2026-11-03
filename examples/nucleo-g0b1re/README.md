# NUCLEO-G0B1RE

This is the board-level system profile for ST's NUCLEO-G0B1RE, built around
the STM32G0B1RET6. The board maps LD4 to PA5, B1 to PC13, and the ST-LINK
virtual COM port to USART2 on PA2/PA3.

The smoke reuses `firmware-stm32g0-demo`: it enables the G0 GPIO and USART2
clock gates, prints `OK` through the VCP, and toggles LD4. The assertion checks
both the UART output and at least eight PA5 edges. It does not claim USB,
ST-LINK debug transport, or the chip's unlisted peripherals.

## Reproduce

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-g0b1re \
  --crate firmware-stm32g0-demo \
  --target thumbv6m-none-eabi \
  --script examples/nucleo-g0b1re/io-smoke.yaml \
  --system configs/systems/nucleo-g0b1re.yaml \
  --out-dir examples/nucleo-g0b1re/evidence
```

The committed `evidence/onboarding-metrics.json` and
`evidence/logs/run_smoke.log` record the run. `evidence/onboarding-scoreboard.md`
is generated from that metric.

## Model boundary

The chip slice includes RCC, SysTick, GPIOA/B/C, USART1/2, TIM2, EXTI, NVIC,
and a SYSCFG stub. The smoke exercises only GPIOA/PA5 and USART2. SYSCFG is a
zero-answering stub; I2C, SPI, ADC, DMA, FDCAN, USB, PLL operation, and GPIO
ports D/F are not claimed by this profile. No NUCLEO-G0B1RE silicon capture is
recorded, so this is simulator evidence rather than hardware confirmation.

## Vendor status checked 2026-11-03

ST's live [NUCLEO-G0B1RE product page](https://www.st.com/en/evaluation-tools/nucleo-g0b1re.html)
states **Active** and “Product is in volume production.” ST's
[eStore listing](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-g0b1re.html)
showed **In Stock** and **Add to Cart**. ST's
[STM32G0B1RE product page](https://www.st.com/en/microcontrollers-microprocessors/stm32g0b1re.html)
also states **Active** / “Product is in volume production.”

See [the board validation page](../../docs/boards/nucleo-g0b1re.md) for
vendor status, source documents, smoke evidence, and the model boundary.
