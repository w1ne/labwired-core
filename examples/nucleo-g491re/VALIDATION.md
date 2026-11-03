# NUCLEO-G491RE validation

The local LabWired engine smoke builds the bare-metal firmware against the
focused STM32G491 descriptor, then checks the default LPUART1 virtual COM path
and repeated LD2/PA5 GPIO transitions.

Reproduce it with:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-g491re \
  --crate firmware-nucleo-g491re-smoke \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-g491re/onboarding-smoke.yaml \
  --system configs/systems/nucleo-g491re.yaml \
  --out-dir examples/nucleo-g491re/evidence
```

The committed result reports `PASS 3/3 checks`. See `evidence/logs/` for the
build and smoke logs and `evidence/simulation-output/` for the UART capture,
GPIO assertion, and JUnit result. The onboarding scoreboard is in
`evidence/onboarding-scoreboard.md`.

The descriptor exposes only modeled RCC, GPIOA, and LPUART1. USB, the user
button, other MCU peripherals, and other board functions are omitted. This is
local simulator evidence, not a hosted `labwired_verify` verdict or a hardware
capture.
