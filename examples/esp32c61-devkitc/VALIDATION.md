# Validation — ESP32-C61-DevKitC-1 v2.0

Run the onboarding smoke from the repository root:

```sh
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32c61-devkitc \
  --crate firmware-esp32c61-devkitc-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/esp32c61-devkitc/onboarding-smoke.yaml \
  --system configs/systems/esp32c61-devkitc.yaml \
  --out-dir examples/esp32c61-devkitc/evidence
```

The smoke requires the UART marker, at least 48 GPIO8 transitions, and a red
pixel (`RGB565 0xF800`) decoded from a 24-bit GRB waveform. It then stops at
the configured instruction-step limit. Build and simulation logs are committed
under [`evidence/`](evidence/). This is local simulator evidence, not a
physical-board capture or hosted `model_verified` result.

The C61 LEDC servo smoke exercises GPIO6/J1 pin 5 and the committed channel 0
duty:

```sh
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32c61-devkitc-ledc-servo \
  --crate firmware-esp32c61-devkitc-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/esp32c61-devkitc/ledc-servo-smoke.yaml \
  --system configs/systems/esp32c61-devkitc.yaml \
  --out-dir examples/esp32c61-devkitc/evidence/ledc-servo
```

It gates on the UART marker, the channel 0 `DUTY_R` readback, at least two
GPIO6 transitions, and the configured step-limit stop. The local smoke does
not assert servo-angle telemetry.
