# NUCLEO-WB55RG validation

The onboarding run builds the repository CLI and the target firmware, then
runs `examples/nucleo-wb55rg/io-smoke.yaml` against
`configs/systems/mb1355c.yaml`:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-wb55rg \
  --crate firmware-nucleo-wb55rg-demo \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-wb55rg/io-smoke.yaml \
  --system configs/systems/mb1355c.yaml \
  --out-dir examples/nucleo-wb55rg/evidence
```

Acceptance requires the USART1 `NUCLEO-WB55RG ready` marker, ordered
readback messages for LED1 and LED2, at least two GPIO edges on each of PB5,
PB0, and PB1, and the deterministic max-step stop. This validates the
simulator model, not a physical board or USB/ST-LINK behavior.

Committed build and run logs, raw simulation output, and generated onboarding
metrics and scoreboard are in [`evidence/`](evidence/). Vendor lifecycle and
purchase evidence is in [`evidence/vendor-status.md`](evidence/vendor-status.md).
