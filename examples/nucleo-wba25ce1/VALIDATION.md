# NUCLEO-WBA25CE1 validation

This profile is simulator-derived. The local firmware smoke checks the modeled
RCC clock-enable path and GPIO v2 output transitions on PA7, wired to LD1. It
does not validate physical silicon, radio operation, USB/ST-LINK, or any
peripheral omitted from this narrow board profile.

Run from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-wba25ce1 \
  --crate firmware-nucleo-wba25ce1-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/nucleo-wba25ce1/onboarding-smoke.yaml \
  --system configs/systems/nucleo-wba25ce1-smoke.yaml \
  --out-dir examples/nucleo-wba25ce1/evidence
```

Acceptance checks: at least eight GPIOA pin 7 edges and the expected max-step
stop. Build and simulator logs plus result artifacts are committed under
`evidence/`.

The hosted verifier catalog has no NUCLEO-WBA25CE1 / STM32WBA25CE target, so
this round makes no hosted `model_verified` claim. See
[`evidence/labwired-verification.md`](evidence/labwired-verification.md).
