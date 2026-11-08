# Seeed Studio XIAO ESP32-S3 GPIO smoke

This profile runs a direct-entry ESP32-S3 firmware image and toggles the
standard XIAO ESP32-S3's onboard active-low user LED on GPIO21. The chip
descriptor exposes the real ESP32-S3 GPIO model used by this test.

The profile covers only that LED path. It omits the Sense and Plus variants,
Wi-Fi/Bluetooth, USB, battery and charger behavior, PSRAM, and all other
header peripherals. The ELF starts directly in the simulator; this smoke does
not claim ESP32-S3 mask-ROM or Arduino framework startup, nor physical-board
testing.

Run from the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id seeed-xiao-esp32s3 \
  --script examples/seeed-xiao-esp32s3/onboarding-smoke.yaml \
  --system configs/systems/seeed-xiao-esp32s3.yaml \
  --firmware-build-script examples/seeed-xiao-esp32s3/ci/build-firmware.sh \
  --out-dir examples/seeed-xiao-esp32s3/evidence
```

The smoke requires eight GPIO21 transitions and records the build and
simulation output under [`evidence/`](evidence/). Vendor lifecycle, stock, and
pin-map evidence is in [`evidence/vendor-status.md`](evidence/vendor-status.md).
