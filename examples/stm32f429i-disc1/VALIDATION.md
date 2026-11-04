# STM32F429I-DISC1 validation

The smoke builds a Cortex-M4 image and runs it with the narrow
`stm32f429i-disc1` profile:

```bash
scripts/onboarding_smoke.sh \
  --target-id stm32f429i-disc1 \
  --crate firmware-stm32f429i-disc1-smoke \
  --target thumbv7em-none-eabi \
  --script examples/stm32f429i-disc1/onboarding-smoke.yaml \
  --system configs/systems/stm32f429i-disc1.yaml \
  --out-dir examples/stm32f429i-disc1/evidence
```

The assertion requires at least 16 GPIOG pin 13 transitions and normal
termination at the configured instruction limit. Firmware enables GPIOG using
the STM32F4 RCC AHB1 gate, configures PG13 as an output, and toggles it through
GPIOG BSRR. ST UM1670 Rev 6 maps green user LED LD3 to PG13; the ST BSP's
`BSP_LED_On` implementation sets that output high.

The smoke profile contains no stub windows. It omits the display, external
SDRAM, gyroscope, USB, buttons and ST-LINK/VCP. The result is local simulator
evidence and makes no physical-board claim.
