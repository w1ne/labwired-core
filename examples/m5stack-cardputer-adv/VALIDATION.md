# Validation runbook

From the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id m5stack-cardputer-adv \
  --script examples/m5stack-cardputer-adv/onboarding-smoke.yaml \
  --system configs/systems/m5stack-cardputer-adv.yaml \
  --firmware-build-script examples/m5stack-cardputer-adv/ci/build-firmware.sh \
  --out-dir examples/m5stack-cardputer-adv/evidence
```

The smoke requires the UART marker `M5STACK_CARDPUTER_ADV_ST7789_OK`, a lit
panel, and 64 red pixels in the 8×8 display region at the bottom of the
controller-oriented glass crop. The runner stops at the configured step cap.
The generated build logs, simulator output, and onboarding metrics are
committed under `evidence/`.
