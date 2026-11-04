# Teensy 4.0 validation

Reproduce the local LabWired engine smoke from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id teensy-40 \
  --crate firmware-teensy4-smoke \
  --target thumbv7em-none-eabi \
  --script examples/teensy-40/onboarding-smoke.yaml \
  --system configs/systems/teensy-40.yaml \
  --out-dir examples/teensy-40/evidence
```

The acceptance check requires at least eight GPIO2 pin 3 transitions and a
normal stop at the configured simulator step limit. The image runs from the
profile's DTCM slice. This smoke does not exercise the external flash/XIP boot
path, IOMUXC pad routing, or peripherals beyond CCM and GPIO2.
