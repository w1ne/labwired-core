# Validation — NUCLEO-F103RB + BME280

**Status: local simulator smoke passed on 2026-11-10.** This run boots the
committed Cortex-M3 firmware with the narrow Nucleo profile and BME280 attached
to I2C1. It is not a physical-board test or a hosted `labwired_verify` result.

## Build and smoke

Reproduce from the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id nucleo-f103rb-bme280 \
  --crate firmware-nucleo-f103rb-bme280-smoke \
  --target thumbv7m-none-eabi \
  --script examples/nucleo-f103rb-bme280/onboarding-smoke.yaml \
  --system configs/systems/nucleo-f103rb-bme280.yaml \
  --out-dir examples/nucleo-f103rb-bme280/evidence
```

Recorded result:

- Build and smoke status: **pass**.
- Smoke assertions: **3/3**; USART2 reports BME280 ID `0x60`, LD2 on PA5
  records at least four transitions, and the run reaches the requested
  200,000-step stop.
- The loaded chip descriptor is `stm32f103rb-nucleo-smoke`. The board profile
  attaches a BME280 at I2C1 address `0x77`.

The committed `evidence/` directory contains the build logs, UART output,
structured simulation result, snapshot, metrics, and scoreboard. The hosted
catalog does not expose an exact NUCLEO-F103RB verifier target; see
[`evidence/labwired-verification.md`](evidence/labwired-verification.md).
