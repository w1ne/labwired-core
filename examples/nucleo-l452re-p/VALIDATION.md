# NUCLEO-L452RE-P validation

This profile is simulator-derived. It uses the STM32L4 RCC, GPIO v2 and LPUART
models to assert the board's default LPUART1 VCP byte path and LD4 PB13 GPIO
edges. The smoke does not validate silicon, USB/ST-LINK behavior, or omitted
board peripherals.

Run from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-l452re-p \
  --crate firmware-nucleo-l452re-p-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-l452re-p/onboarding-smoke.yaml \
  --system configs/systems/nucleo-l452re-p-smoke.yaml \
  --out-dir examples/nucleo-l452re-p/evidence
```

The acceptance checks are the `NUCLEO_L452RE_P_OK` LPUART marker, at least
eight LD4 GPIO edges, and the expected max-step stop. Logs, result JSON, and
metrics are committed under `evidence/` after the run.
