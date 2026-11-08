# NUCLEO-F722ZE

This board profile boots a Cortex-M7 image on the STM32F722ZET6 and exercises
the active-high green user LED LD1, routed from GPIOB pin 0 when SB120 is ON
and SB119 is OFF. The profile exposes only RCC and GPIOB for this smoke;
it does not claim the board's other MCU or external peripherals.

Reproduce from the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id nucleo-f722ze \
  --crate firmware-nucleo-f722ze-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-f722ze/onboarding-smoke.yaml \
  --system configs/systems/nucleo-f722ze.yaml \
  --out-dir examples/nucleo-f722ze/evidence
```

The committed evidence contains build logs, the simulator smoke result, and
onboarding metrics. ST's lifecycle, purchase, and wiring sources are recorded
in [`evidence/vendor-status.md`](evidence/vendor-status.md).
