# NUCLEO-H7A3ZI-Q

This focused profile boots the STM32H7A3ZIT6Q firmware and toggles the board's
active-high user LED LD1 on PB0. The PB0 route uses the UM2408 configuration
SB54 ON / SB65 OFF. The profile exposes only the reset-clock RCC path, GPIOB,
the flash image, and the DTCM used by the smoke. It does not claim support for
the other SRAM banks, ST-LINK, serial, buttons, expansion headers, or other MCU
peripherals; it declares no zero-answering stub peripherals.

ST's live store listing marked the NUCLEO-H7A3ZI-Q **Active** and **In Stock**,
with **Add to Cart**. The exact fitted STM32H7A3ZIT6Q is marked **Active** but
was **Out of Stock** as a separate component; the board itself was orderable.
See [the vendor-status receipt](evidence/vendor-status.md).

## Smoke evidence

The local onboarding smoke passed **2/2** assertions at 20,000 steps. It
recorded 32 PB0 GPIO edges and stopped at the expected max-step limit. Build
logs, simulation output, metrics, and scoreboard are committed under
[`evidence/`](evidence/). The run assumes an ideal VDD rail, so power-on and
brown-out reset behavior are not exercised.

The hosted LabWired catalog currently has no H7A3 target, so this is local
engine evidence and does not carry a hosted `model_verified` verdict or a
physical-board capture. The H7 RCC model is reused from the engine's H7 family
profile; this exact H7A3 profile has not been checked against physical silicon.

## Reproduce

From the repository root:

```bash
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-h7a3zi-q \
  --crate firmware-nucleo-h7a3zi-q-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-h7a3zi-q/onboarding-smoke.yaml \
  --system configs/systems/nucleo-h7a3zi-q.yaml \
  --out-dir examples/nucleo-h7a3zi-q/evidence
```
