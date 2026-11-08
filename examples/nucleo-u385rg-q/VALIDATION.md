# Validation record

The committed local smoke was generated from the repository root with:

```sh
scripts/onboarding_smoke.sh \
  --target-id nucleo-u385rg-q \
  --crate firmware-nucleo-u385rg-q-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/nucleo-u385rg-q/onboarding-smoke.yaml \
  --system configs/systems/nucleo-u385rg-q-smoke.yaml \
  --out-dir examples/nucleo-u385rg-q/evidence
```

The local CLI smoke output records `PASS 2/2 checks`, reaches the configured
5,000-step limit, and captures 31 transitions on GPIOA pin 5 with no logic-edge
gaps. See `evidence/logs/` and `evidence/simulation-output/result.json` for the
build, assertion, and simulator records.

The public hosted LabWired catalog currently has no `NUCLEO-U385RG-Q` board or
STM32U3 MCU entry, so this record does not claim a hosted `labwired_verify`
oracle verdict. The board smoke remains scoped to the local repository model.
