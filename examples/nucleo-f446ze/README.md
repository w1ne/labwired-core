# NUCLEO-F446ZE smoke

This focused Nucleo-144 profile exercises the STM32F446ZET6's USART3 VCP path
and its three on-board user LEDs. The profile exposes only RCC, GPIOB, GPIOD,
and USART3; the board's other features are not claimed.

Run the smoke from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-f446ze \
  --crate firmware-f446-demo \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-f446ze/onboarding-smoke.yaml \
  --system configs/systems/nucleo-f446ze.yaml \
  --out-dir examples/nucleo-f446ze/evidence
```

The committed build logs and simulator results are under `evidence/`.
