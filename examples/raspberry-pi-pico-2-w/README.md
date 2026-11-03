# Raspberry Pi Pico 2 W (RP2350)

This onboarding target covers the exposed GP0 header pad through RP2350
IO_BANK0 and SIO. The smoke firmware selects SIO for GP0 and asserts at least
eight output edges on the simulated pin.

## Run

```bash
scripts/onboarding_smoke.sh \
  --target-id raspberry-pi-pico-2-w \
  --crate firmware-pico2-w-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/raspberry-pi-pico-2-w/io-smoke.yaml \
  --system configs/systems/raspberry-pi-pico-2-w.yaml \
  --firmware-build-dir crates/firmware-pico2-w-smoke \
  --out-dir examples/raspberry-pi-pico-2-w/evidence
```

The committed `evidence/` directory contains the build and smoke logs. The live
vendor availability and product-status record is in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Scope

The profile claims only the RP2350 GP0 header GPIO path. It omits the
CYW43439-controlled wireless functions and onboard LED, USB, and other
peripherals; see [`KNOWN_LIMITATIONS.md`](KNOWN_LIMITATIONS.md).
