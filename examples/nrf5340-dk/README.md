# Nordic nRF5340 DK (PCA10095)

This board smoke runs the application-core firmware on the nRF5340 and checks
its on-board user LEDs and the UARTE0 console. The firmware lights LED1 through
LED4 in sequence, reading back the GPIO OUT register before printing each
state. The board map follows Nordic's PCA10095 assignments: LEDs on P0.28–P0.31
(active low) and the VCOM0 TX path on UARTE0/P0.20. The customer-facing system
uses [`nrf5340-dk-smoke.yaml`](../../configs/chips/nrf5340-dk-smoke.yaml), which
exposes only the modeled GPIO0 and UARTE0 paths exercised here.

Reproduce from the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id nrf5340dk \
  --crate firmware-nrf5340-dk-demo \
  --target thumbv8m.main-none-eabi \
  --script examples/nrf5340-dk/io-smoke.yaml \
  --system configs/systems/nrf5340dk.yaml \
  --out-dir examples/nrf5340-dk/evidence
```

Committed build and simulator artifacts are in [`evidence/`](evidence/).
Current board availability is recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

This smoke covers GPIO0 LED outputs and UARTE0 TX. It does not claim working
radio, NFC, network-core firmware, QSPI, button input, or TrustZone enforcement.
The broader Zephyr boot profile in `configs/chips/nrf5340.yaml` retains
zero-answering setup/trim windows for DCNF, FPU, cache, SPU,
oscillators/regulators, CTRL-AP, GPIOTE0, FICR, UICR, and analog trim. Those
windows are not exposed by the customer-facing smoke profile.
