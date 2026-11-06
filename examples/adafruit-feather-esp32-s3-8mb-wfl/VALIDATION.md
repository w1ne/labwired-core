# Validation runbook

From the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id adafruit-feather-esp32-s3-8mb-wfl \
  --firmware-build-script examples/adafruit-feather-esp32-s3-8mb-wfl/ci/build-firmware.sh \
  --script examples/adafruit-feather-esp32-s3-8mb-wfl/onboarding-smoke.yaml \
  --system configs/systems/adafruit-feather-esp32-s3-8mb-wfl.yaml \
  --out-dir examples/adafruit-feather-esp32-s3-8mb-wfl/evidence
```

The smoke runs compiled ESP32-S3 firmware against the local engine and checks
the MAX17048 VCELL and SOC words on UART0, followed by the configured step cap.
The committed compiler logs, simulator output, and onboarding metrics are under
`evidence/`. This does not test physical hardware or establish a hosted oracle
`model_verified` result for the MAX17048.
