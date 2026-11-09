# NUCLEO-G491RE validation

The local LabWired engine smoke builds the bare-metal firmware against the
focused STM32G491 descriptor. It checks the default LPUART1 virtual COM path,
repeated LD2/PA5 GPIO transitions, and an FDCAN1 internal-loopback frame. The
firmware checks the received ID, DLC, and all eight data bytes.

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

The committed result reports `PASS 4/4 checks`. See `evidence/logs/` for the
build and smoke logs and `evidence/simulation-output/` for the UART capture,
GPIO assertion, and JUnit result. The onboarding scoreboard is in
`evidence/onboarding-scoreboard.md`.

The descriptor exposes only modeled RCC, GPIOA, LPUART1, and FDCAN1. The board
does not include a CAN transceiver; the loopback check does not verify PA11/PA12
pad propagation, bit timing, arbitration, or an external CAN bus. USB, the user
button, and other MCU peripherals are omitted. This is local simulator evidence,
not a hosted `labwired_verify` verdict or a hardware capture. The hosted board
catalog does not currently list `nucleo-g491re`.
