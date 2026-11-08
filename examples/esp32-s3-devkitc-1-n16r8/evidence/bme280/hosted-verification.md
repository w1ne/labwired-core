# Hosted firmware verification

Checked 2026-11-06 UTC with `labwired_pcb_labwired_compile` and
`labwired_pcb_labwired_verify` for `esp32-s3-devkitc-1-n16r8`.

- The Arduino sketch compiled successfully with no diagnostics and returned
  ESP32-S3 flash images.
- The hosted run returned `failed`, not `model_verified`. Neither BME280 serial
  oracle clause passed. The run used all 1,500,000 steps and stalled on repeated
  reads of `uart0_s3` register `0x1c`, waiting for a status bit that did not
  change; the verifier advised that a larger step budget would not help.
- The hosted result reported the existing ideal-VDD approximation because the
  verification diagram does not route board power. It did not report an
  unmodeled peripheral fault.

The committed local onboarding smoke remains a separate result: it boots the
fast-boot register-level firmware on the repository simulator and passes all
three assertions. See `simulation-output/result.json` and `logs/run_smoke.log`.
The hosted Arduino run is recorded as a failure and is not evidence of hosted
model verification.
