# Validation — M5Stack AtomS3-Lite

Run the local smoke from the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id m5stack-atoms3-lite \
  --firmware-build-script examples/m5stack-atoms3-lite/ci/build-firmware.sh \
  --script examples/m5stack-atoms3-lite/onboarding-smoke.yaml \
  --system configs/systems/m5stack-atoms3-lite.yaml \
  --out-dir examples/m5stack-atoms3-lite/evidence
```

The smoke asserts GPIO35 edges, the decoded red NeoPixel, and the configured
instruction-limit stop. The committed logs and generated metrics are under
`evidence/`. This validates the local S3 model on the exact narrow board
descriptor; it is not hosted LabWired verification or a physical-board test.
