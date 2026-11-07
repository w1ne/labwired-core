# Validation — ESP32-C3-DevKit-RUST-2

Run the onboarding smoke from the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id esp32c3-devkit-rust-2 \
  --crate firmware-esp32c3-rust2-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/esp32c3-devkit-rust-2/onboarding-smoke.yaml \
  --system configs/systems/esp32c3-devkit-rust-2.yaml \
  --out-dir examples/esp32c3-devkit-rust-2/evidence
```

The pass condition requires the firmware to read a six-byte SHTC3 measurement
with both CRCs valid, at least 48 GPIO2 edges, a decoded red pixel (`RGB565
0xF800`) from the 24-bit GRB frame, and the configured maximum-step stop
reason. Logs and generated metrics are committed under `evidence/`.

This repository smoke runs the local simulator against the exact narrow board
profile. It is distinct from hosted LabWired verification and from physical
hardware testing; see [`evidence/labwired-verification.md`](evidence/labwired-verification.md).
