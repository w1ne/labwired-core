# Validation — NUCLEO-F103RB

**Status: simulator smoke passed on 2026-11-03.** This run uses the committed
`configs/systems/nucleo-f103rb.yaml` profile and the
`firmware-nucleo-f103rb-smoke` binary. It is not a physical-board test.

## Build and smoke

Reproduce the committed run from the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id nucleo-f103rb \
  --crate firmware-nucleo-f103rb-smoke \
  --target thumbv7m-none-eabi \
  --script examples/nucleo-f103rb/io-smoke.yaml \
  --system configs/systems/nucleo-f103rb.yaml \
  --out-dir examples/nucleo-f103rb/evidence
```

Recorded result:

- Build and smoke status: **pass**.
- Firmware SHA-256: `6fecbf569312c1cf2dffa2f62aacf82cab83972490919016b74a24f396d39739`.
- Smoke assertions: **3/3**; USART2 output contains `OK`, GPIOA pin 5 records
  **10 transitions**, and the run stops at the requested 200,000 steps.
- The loaded chip descriptor is `stm32f103rb-nucleo-smoke`; it declares RCC,
  GPIOA, GPIOC, and USART2 only.

The committed `evidence/` directory contains the build logs, UART transcript,
simulation result, snapshot, metrics, and scoreboard. Physical UART and LED
parity have not been captured. The B1 button is mapped but not exercised.
