# Validation

Run from the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id heltec-wifi-lora-32-v4-s3r2 \
  --firmware-build-script examples/heltec-wifi-lora-32-v4-s3r2/ci/build-firmware.sh \
  --script examples/heltec-wifi-lora-32-v4-s3r2/onboarding-smoke.yaml \
  --system configs/systems/heltec-wifi-lora-32-v4-s3r2.yaml \
  --out-dir examples/heltec-wifi-lora-32-v4-s3r2/evidence
```

The smoke requires at least eight GPIO35 edges before its step cap. Build logs,
simulator output, and the onboarding metrics are committed under `evidence/`.
