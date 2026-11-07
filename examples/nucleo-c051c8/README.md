# NUCLEO-C051C8

This is a narrow first system profile for ST's NUCLEO-C051C8. The local smoke
builds and runs an STM32C051C8 image, enables GPIOA, and observes transitions
on the board's active-high LD4 user LED (PA5).

The descriptor claims only reset-clock boot, RCC IOPENR GPIOA clock enable,
GPIOA, and LD4. Buttons, VCP/UART, USB, and expansion connectors are not modeled
by this profile. Power rails are idealized by the engine, so this smoke does not
verify power-on or brown-out behavior. Evidence is local LabWired simulation,
not a physical-board capture. The hosted `labwired_verify` catalog does not yet
contain NUCLEO-C051C8.

Run the committed onboarding smoke with:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id nucleo-c051c8 \
  --crate firmware-nucleo-c051c8-smoke \
  --target thumbv6m-none-eabi \
  --script examples/nucleo-c051c8/onboarding-smoke.yaml \
  --system configs/systems/nucleo-c051c8.yaml \
  --out-dir examples/nucleo-c051c8/evidence
```

The committed logs and simulation output live under [`evidence/`](evidence/).
Vendor status at onboarding time is recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).
