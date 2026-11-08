# STM32F103C8T6 Blue Pill

This system models the STM32F103C8T6 Blue Pill's onboard PC13 LED. The MCU is
the survey's **rank 4** MCU target (STM32F103, 17% in JetBrains State of
Embedded 2025). The board schematic wires its user LED from 3.3 V through a
resistor to PC13, so the LED is active-low.

## Smoke scope

The firmware enables GPIOC, configures PC13 as a push-pull output, and toggles
the LED. The narrow chip profile declares only RCC and GPIOC. It does not claim
working USB, UART, ADC, timer, SPI, I2C, or CAN behavior.

## Build and run

From the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id stm32f103c8-bluepill \
  --crate firmware-stm32f103c8-bluepill-smoke \
  --target thumbv7m-none-eabi \
  --script examples/stm32f103c8-bluepill/io-smoke.yaml \
  --system configs/systems/stm32f103c8-bluepill.yaml \
  --out-dir examples/stm32f103c8-bluepill/evidence
```

The committed `evidence/` directory records the build, GPIO edge assertion,
simulation result, and metrics. Vendor and board availability are recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).
