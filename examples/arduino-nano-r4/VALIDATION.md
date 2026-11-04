# Arduino Nano R4 validation

Run from the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id arduino-nano-r4 \
  --crate firmware-arduino-nano-r4-smoke \
  --target thumbv7em-none-eabi \
  --script examples/arduino-nano-r4/onboarding-smoke.yaml \
  --system configs/systems/arduino-nano-r4.yaml \
  --out-dir examples/arduino-nano-r4/evidence
```

The smoke passes when the run captures at least 16 edges on `port2:4` and
reaches the expected 16,000-step limit. This checks the GPIO writes and the
board pin connection in the simulator; it is not a physical hardware test.

The profile includes only the RA4M1 GPIO route to P204 used by this firmware.
See [`evidence/vendor-status.md`](evidence/vendor-status.md) for the live
manufacturer lifecycle and purchase check.
