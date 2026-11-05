# Validation runbook

From the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id adafruit-esp32-s3-tft-feather \
  --script examples/adafruit-esp32-s3-tft-feather/onboarding-smoke.yaml \
  --system configs/systems/adafruit-esp32-s3-tft-feather.yaml \
  --firmware-build-script examples/adafruit-esp32-s3-tft-feather/ci/build-firmware.sh \
  --out-dir examples/adafruit-esp32-s3-tft-feather/evidence
```

The smoke requires the UART marker
`ADAFRUIT_ESP32S3_TFT_FEATHER_ST7789_OK`, a lit panel, and 64 red pixels in the
declared 8×8 display region. The runner then stops at the configured step cap.
The logs under `evidence/logs/` and the output under
`evidence/simulation-output/` are committed with the profile.
