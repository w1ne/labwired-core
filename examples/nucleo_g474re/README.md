# NUCLEO-G474RE

Board profile for ST's NUCLEO-G474RE, built around the STM32G474RET6. The
system manifest maps the default STLINK-V3E virtual COM port to LPUART1 on
PA2/PA3, LD2 to PA5, and B1 to PC13. These assignments follow ST's [UM2505
board manual](https://www.st.com/resource/en/user_manual/um2505-stm32g4-nucleo64-boards-mb1367-stmicroelectronics.pdf).

The smoke boots a bare-metal Cortex-M4 image, enables the GPIOA and LPUART1
clock gates, prints `NUCLEO_G474RE_OK` through LPUART1, and toggles LD2 four
times. Its assertions check the UART transcript and at least eight PA5 edges.
This is simulator evidence; it is not a hardware capture.

## Reproduce

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-g474re \
  --crate firmware-nucleo_g474re-demo \
  --firmware-build-dir examples/nucleo_g474re/board_firmware \
  --target thumbv7em-none-eabi \
  --script examples/nucleo_g474re/uart-smoke.yaml \
  --system configs/systems/nucleo_g474re.yaml \
  --out-dir examples/nucleo_g474re/evidence
```

The committed `evidence/logs/run_smoke.log` and
`evidence/simulation-output/result.json` capture the run and its UART/GPIO
assertions. To refresh the one-board scoreboard after a new run:

```bash
python3 scripts/generate_onboarding_scoreboard.py \
  --metrics-root examples/nucleo_g474re/evidence \
  --markdown-out examples/nucleo_g474re/evidence/onboarding-scoreboard.md \
  --json-out examples/nucleo_g474re/evidence/onboarding-scoreboard.json
```

## Model boundary

The chip profile declares RCC, GPIOA-D, SysTick, USART1/2, LPUART1, FLASH, PWR,
CRC, TIM1/2, I2C1, SPI1, ADC1, DMA1, IWDG, RTC, NVIC, and DBGMCU. This smoke
exercises only boot, RCC clock enables, GPIOA/PA5, and LPUART1. ADC1 models its
power-up path but does not provide an analog conversion engine. The other
declared peripherals are outside this smoke's claim. The chip/system descriptor
does not declare any `type: stub` zero-answering windows. The simulated UART
transcript is captured from the LPUART1 model; no UART pad waveform or silicon
measurement is included.

## Vendor status checked 2026-11-03

ST's live [NUCLEO-G474RE product page](https://www.st.com/en/evaluation-tools/nucleo-g474re.html)
states **Active** and “Product is in volume production.” ST's live [eStore
listing](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-g474re.html)
showed **In Stock** and **Add to Cart**. ST's [STM32G474RE product
page](https://www.st.com/en/microcontrollers-microprocessors/stm32g474re.html)
also states **Active** and “Product is in volume production.”

See [the G474 board validation page](../../docs/boards/stm32g474re.md) for
existing validation coverage and remaining model limits.
