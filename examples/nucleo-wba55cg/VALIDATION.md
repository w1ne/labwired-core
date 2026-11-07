# NUCLEO-WBA55CG validation

The onboarding profile is simulator-derived. It uses the STM32WBA RCC, GPIO
v2, and USART v2 models to assert the default VCP1 USART1 byte path and LD1
PB4 GPIO edges. It does not validate physical silicon, USB/ST-LINK behavior,
the integrated radio, or peripherals omitted from the narrow profile.

Run from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-wba55cg \
  --crate firmware-nucleo-wba55cg-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/nucleo-wba55cg/onboarding-smoke.yaml \
  --system configs/systems/nucleo-wba55cg-smoke.yaml \
  --out-dir examples/nucleo-wba55cg/evidence
```

The acceptance checks are the `NUCLEO_WBA55CG_OK` USART1 marker, at least eight
LD1 GPIO edges, and the expected max-step stop. Build logs, simulator output,
scoreboard, metrics, and the vendor status receipt are committed under
`evidence/`.
