# Teensy 4.1 validation

Reproduce the local LabWired engine smoke from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id teensy-41 \
  --crate firmware-teensy41-smoke \
  --target thumbv7em-none-eabi \
  --script examples/teensy-41/onboarding-smoke.yaml \
  --system configs/systems/teensy-41.yaml \
  --out-dir examples/teensy-41/evidence
```

The acceptance check requires at least eight GPIO2 pin 3 transitions and a
normal stop at the configured simulator step limit. This is local model
evidence; there is no physical-board capture. Serial, IOMUXC routing,
FlexSPI/XIP, USB, Ethernet, SD, and external PSRAM are outside this profile.
