# QT Py RP2040 + BME280 validation

Run from the repository root:

```sh
rustup target add thumbv6m-none-eabi  # once
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id qtpy-rp2040-bme280 \
  --crate firmware-qtpy-rp2040-bme280-smoke \
  --target thumbv6m-none-eabi \
  --script examples/qtpy-rp2040-bme280/bme280-smoke.yaml \
  --system configs/systems/qtpy-rp2040-bme280.yaml \
  --out-dir examples/qtpy-rp2040-bme280/evidence
```

The passing smoke requires BME280 ID `0x60`, raw temperature `0x082DC0`, the
UART1 success marker, and the expected max-step stop. Committed logs and the
simulation result are under [`evidence/`](evidence/). The verdict is simulator
evidence, not hardware confirmation.
