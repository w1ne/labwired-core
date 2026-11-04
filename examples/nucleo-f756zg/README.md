# NUCLEO-F756ZG

This board profile boots firmware on the STM32F756ZGT6 and exercises two
signals wired on the Nucleo-144 board: USART3 through the ST-LINK virtual COM
port on PD8/PD9, and the active-high blue user LED LD2 on PB7. Its system and
chip descriptors contain only the RCC, GPIOB/GPIOD, USART3, and SysTick models
needed by this smoke.

Reproduce from the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id nucleo-f756zg \
  --crate firmware-nucleo-f756zg-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-f756zg/onboarding-smoke.yaml \
  --system configs/systems/nucleo-f756zg.yaml \
  --out-dir examples/nucleo-f756zg/evidence
```

Committed build and simulator artifacts are under [`evidence/`](evidence/).
ST's live lifecycle and purchase check is recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

The F756ZG also carries Ethernet, USB OTG, and cryptographic capabilities.
Those paths are not included in this descriptor or claimed by this smoke.
