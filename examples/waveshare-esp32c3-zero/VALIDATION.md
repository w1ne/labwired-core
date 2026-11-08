# Validation — Waveshare ESP32-C3-Zero

Run the focused board smoke from the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id waveshare-esp32c3-zero \
  --crate firmware-esp32c3-waveshare-c3-zero-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/waveshare-esp32c3-zero/onboarding-smoke.yaml \
  --system configs/systems/waveshare-esp32c3-zero.yaml \
  --out-dir examples/waveshare-esp32c3-zero/evidence
```

The assertions require at least 48 transitions on GPIO10 and a decoded red
pixel (`RGB565 0xF800`) from the WS2812 frame. The smoke reaches the configured
step limit after emitting the frame. Build and run logs plus onboarding metrics
are committed in `evidence/`.
