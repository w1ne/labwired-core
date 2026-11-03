# NUCLEO-F446ZE validation

The local LabWired engine builds bare-metal firmware and runs it against the
focused NUCLEO-F446ZE profile. The smoke checks USART3's VCP banner and at least
eight GPIO transitions on each of LD1/PB0, LD2/PB7, and LD3/PB14.

Reproduce it with:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-f446ze \
  --crate firmware-f446-demo \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-f446ze/onboarding-smoke.yaml \
  --system configs/systems/nucleo-f446ze.yaml \
  --out-dir examples/nucleo-f446ze/evidence
```

The run must pass all five assertions. See `evidence/logs/` for compiler and
smoke logs and `evidence/simulation-output/` for UART, GPIO, and JUnit output.
This is local simulator evidence; it is not a hosted `labwired_verify` verdict
or a physical-board capture.
