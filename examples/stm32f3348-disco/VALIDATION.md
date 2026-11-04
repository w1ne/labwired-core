# STM32F3348-DISCO validation

The smoke builds a Cortex-M4 image and runs it with the narrow
`stm32f3348-disco` profile:

```sh
scripts/onboarding_smoke.sh \
  --target-id stm32f3348-disco \
  --crate firmware-stm32f3348-disco-smoke \
  --target thumbv7em-none-eabi \
  --script examples/stm32f3348-disco/onboarding-smoke.yaml \
  --system configs/systems/stm32f3348-disco.yaml \
  --out-dir examples/stm32f3348-disco/evidence
```

Acceptance requires at least 16 transitions on GPIOB pin 6 and a normal stop at
the instruction limit. Firmware enables GPIOB through RCC AHBENR, configures
PB6 as an output, and drives it through GPIOB BSRR. ST UM1735 Rev 3 maps the
active-high red user LED LD3 to PB6. The ST-provided board BSP's
`BSP_LED_On` implementation writes a high level to the LED's GPIO.

The profile does not model the board's other LEDs, button, ST-LINK/VCP,
high-brightness LED driver, or other MCU peripherals. It contains no stub
windows and makes no physical-hardware claim.
