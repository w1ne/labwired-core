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

The radio command smoke is run separately so its evidence remains distinct:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id heltec-wifi-lora-32-v4-s3r2-sx1262 \
  --firmware-build-script examples/heltec-wifi-lora-32-v4-s3r2/ci/build-radio-firmware.sh \
  --script examples/heltec-wifi-lora-32-v4-s3r2/radio-onboarding-smoke.yaml \
  --system configs/systems/heltec-wifi-lora-32-v4-s3r2.yaml \
  --out-dir examples/heltec-wifi-lora-32-v4-s3r2/evidence/sx1262
```

It asserts the LoRa packet type readback and the SX1262 model's TxDone IRQ
through the committed firmware marker. It does not exercise the RF link.
