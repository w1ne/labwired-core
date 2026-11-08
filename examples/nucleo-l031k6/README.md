# NUCLEO-L031K6 onboarding

This focused Nucleo-32 profile boots firmware on the STM32L031K6T6 and toggles
the green user LED LD3, mapped to PB3 / Arduino Nano D13 (CN4.15). The model
exposes only STM32L0 reset-clock RCC and GPIOB/PB3. It omits the ST-LINK VCP,
USB, and the MCU's other peripherals; no zero-answering stubs are declared.

ST's live product page lists the board Active, and ST's store showed it In
Stock with Add to Cart. The checked URLs and board wiring source are recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

Reproduce from the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-l031k6 \
  --crate firmware-nucleo-l031k6-smoke \
  --target thumbv6m-none-eabi \
  --script examples/nucleo-l031k6/onboarding-smoke.yaml \
  --system configs/systems/nucleo-l031k6.yaml \
  --out-dir examples/nucleo-l031k6/evidence
```

The committed build logs, GPIO edge assertion, local simulator output, and
metrics are in [`evidence/`](evidence/). This is local simulator evidence; no
physical-board capture or hosted `labwired_verify` verdict is claimed.
