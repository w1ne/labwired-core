# Waveshare ESP32-S3-Zero (FH4R2)

**Demand rank:** ESP32 is **#1** in the measured MCU-use list at 38%. This round corrects and exercises the existing ESP32-S3-Zero board profile; it is a board-coverage fix, not new chip-family support.

The onboard WS2812 DIN is **GPIO21**. Earlier board metadata incorrectly treated GPIO48 as a simple LED. The corrected system attaches the WS2812 component model to GPIO21, and the smoke sends a GRB `[64, 0, 0]` frame by GPIO timing. A separate artifact check requires the model to decode one lit pixel.

## Reproduce

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id esp32s3-zero-ws2812 \
  --firmware-build-script examples/esp32s3-zero/build-firmware.sh \
  --script examples/esp32s3-zero/ws2812-smoke.yaml \
  --system configs/systems/esp32s3-zero.yaml \
  --out-dir examples/esp32s3-zero/evidence \
  --threshold-seconds 3600
python3 examples/esp32s3-zero/verify_ws2812_artifact.py \
  examples/esp32s3-zero/evidence/simulation-output/result.json
```

The evidence includes build and run logs, the standard smoke result, the WS2812 decoded-pixel check, and the hosted oracle receipt in [`evidence/hosted-model-verification.md`](evidence/hosted-model-verification.md). The local smoke fast-loads the user ELF; the hosted verifier additionally exercises its flash-ROM boot path. Neither verifies physical hardware.

## Model boundary

The exercised path is the S3 GPIO output plus the attached WS2812 model. Wi-Fi, BLE, USB/CDC, and the board's 2 MB PSRAM are outside this profile. On the S3 chip descriptor, SYSTEM, RTC_CNTL, and eFuse remain stubs; this smoke does not touch them. It is simulator evidence, not a physical-board test.

Vendor status and pin references are recorded in [`vendor-status.md`](vendor-status.md).
