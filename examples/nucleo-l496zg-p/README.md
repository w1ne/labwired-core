# NUCLEO-L496ZG-P

This local simulator smoke boots Cortex-M4 firmware for ST's NUCLEO-L496ZG-P
and requires at least 16 transitions on the board's red user LED LD3, wired to
PB14. The profile exposes only the STM32L496 RCC and GPIOB needed for this path.

## Reproduce

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-l496zg-p \
  --crate firmware-nucleo-l496zg-p-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-l496zg-p/onboarding-smoke.yaml \
  --system configs/systems/nucleo-l496zg-p.yaml \
  --out-dir examples/nucleo-l496zg-p/evidence
```

ST's current status and exact purchase variant are recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md). Committed local
simulator output and generated onboarding metrics are under [`evidence/`](evidence/).

## Model boundary

The profile models RCC_AHB2ENR.GPIOBEN and GPIOB's register behavior, and maps
LD3 to PB14. The other user LEDs and button, ST-LINK/VCP, USB, SMPS operation,
header devices, other GPIO ports, and all other MCU peripherals are omitted.
This local simulator evidence does not confirm behavior on physical hardware.
