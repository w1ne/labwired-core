# Validation

Run from the repository root with:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-f070rb \
  --crate firmware-nucleo-f070rb-smoke \
  --target thumbv6m-none-eabi \
  --script examples/nucleo-f070rb/onboarding-smoke.yaml \
  --system configs/systems/nucleo-f070rb.yaml \
  --out-dir examples/nucleo-f070rb/evidence
```

The smoke asserts at least eight transitions on modeled GPIOA pin 5 and the
expected bounded stop. `evidence/logs/run_smoke.log` records the simulator
result; `evidence/simulation-output/result.json` contains the assertion
results and observed GPIO state. The descriptor omits every peripheral other
than the RCC registers needed for GPIOA clocking and GPIOA itself.
