# Nordic nRF52840-DK onboarding

This focused board profile boots the nRF52840 and exercises the DK's active-low
physical LED1 through GPIO P0.13. Zephyr names this board alias `led0`. The
smoke requires at least 16 GPIO transitions.

## Reproduce

From the repository root:

```bash
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nrf52840-dk \
  --crate firmware-nrf52840-dk-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nrf52840-dk/onboarding-smoke.yaml \
  --system configs/systems/nrf52840-dk-led-smoke.yaml \
  --out-dir examples/nrf52840-dk/evidence
```

The committed build and simulation logs are under `evidence/`. The live product
availability check is in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Model boundary

The smoke system exposes GPIO0 and physical LED1 only. The other LEDs and
buttons, radio, NFC, USB, QSPI memory, and all other nRF52840 peripherals are
outside this profile; the test does not claim they are simulated.
