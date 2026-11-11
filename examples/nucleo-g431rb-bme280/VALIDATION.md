# Validation

## Reproduce the local simulator smoke

Run from the repository root:

```bash
rustup target add thumbv7em-none-eabi # once
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-g431rb-bme280 \
  --crate firmware-nucleo-g431rb-bme280-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-g431rb-bme280/bme280-smoke.yaml \
  --system configs/systems/nucleo-g431rb-bme280.yaml \
  --out-dir examples/nucleo-g431rb-bme280/evidence
```

The smoke gates on the BME280 ID and raw-temperature read, I2C1 bus traffic,
PB8/PB9 activity, at least eight PA5 LED edges, and the expected step-limit
stop. The committed logs and `onboarding-metrics.json` are the evidence for this
exact profile. This is local simulator evidence, not silicon confirmation.

## Scope and gaps

- Exact chip geometry is STM32G431RBT6: 128 KiB flash and the descriptor's
  22 KiB main SRAM window; the separate 10 KiB CCM bank remains outside the
  descriptor.
- The smoke uses real modeled I2C1, LPUART1 and GPIO behavior. The BME280 is
  connected directly to the I2C model, so firmware's PB8/PB9 alternate-function
  writes and physical pull-ups are not electrically enforced by this profile.
- No motor-control, analog, USB, ST-LINK firmware, or physical-board behavior is
  claimed.
