# NUCLEO-G474RE

Bare-metal smoke coverage for ST's NUCLEO-G474RE and STM32G474RE. The firmware
prints `OK` through USART2 and configures TIM1 channel 1 to drive a 1 kHz, 50%
PWM signal on PA8/AF6, available on the Arduino D7 header pin. The smoke checks
the UART marker and at least eight PA8 transitions. This is simulator evidence,
not a hardware capture. ST's [UM2505 board manual](https://www.st.com/resource/en/user_manual/um2505-stm32g4-nucleo64-boards-mb1367-stmicroelectronics.pdf)
maps CN9 D7 to PA8.

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
python3 scripts/generate_onboarding_scoreboard.py \
  --metrics-root examples/nucleo_g474re/evidence \
  --markdown-out examples/nucleo_g474re/evidence/onboarding-scoreboard.md \
  --json-out examples/nucleo_g474re/evidence/onboarding-scoreboard.json
```

The firmware is in a standalone Cargo workspace, so `--firmware-build-dir`
builds it from that workspace. Committed logs and the structured smoke result
are under `evidence/`.

## Model boundary

This smoke exercises boot, the USART2 marker path, GPIOA clock enable, TIM1
registers, and the PA8/AF6 timer-output route. The G4 timer output table only
adds TIM1_CH1 on PA8 in this change; other TIM1/TIM2/TIM3 pads and other
declared chip peripherals are outside the smoke claim. It does not establish
physical timing or operation on a hardware board.

## Vendor status checked 2026-11-10

ST's live [NUCLEO-G474RE product page](https://www.st.com/en/evaluation-tools/nucleo-g474re.html)
states **Active** and “Product is in volume production.” The live [ST eStore
listing](https://estore.st.com/en/products/evaluation-tools/product-evaluation-tools/mcu-mpu-eval-tools/stm32-mcu-mpu-eval-tools/stm32-nucleo-boards/nucleo-g474re.html)
states **In Stock** and offers **Add to Cart**. ST's [STM32G474RE product
page](https://www.st.com/en/microcontrollers-microprocessors/stm32g474re.html)
also states **Active** and “Product is in volume production.” The [STM32G474RE
datasheet](https://www.st.com/resource/en/datasheet/stm32g474re.pdf) assigns
PA8/TIM1_CH1 to AF6 (Table 13).
