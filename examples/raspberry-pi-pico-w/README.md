# Raspberry Pi Pico W

This profile adds the SC0918 Pico W as a system using a narrow RP2040 model.
The smoke configures exposed header GPIO GP0 (physical pin 1) and toggles it
through IO_BANK0 and SIO. The system does not claim the wireless module or the
wireless-driven onboard LED.

Run from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id raspberry-pi-pico-w \
  --crate firmware-pico-w-smoke \
  --target thumbv6m-none-eabi \
  --script examples/raspberry-pi-pico-w/io-smoke.yaml \
  --system configs/systems/raspberry-pi-pico-w.yaml \
  --out-dir examples/raspberry-pi-pico-w/evidence
```

The smoke requires at least eight GPIO transitions on `sio:0`. Committed build
and simulation evidence is under [`evidence/`](evidence/). The model result is
engine evidence, not a physical-board capture.

## Scope

- Modeled and exercised: firmware startup from mapped flash, GP0 pin mapping,
  IO_BANK0 function select, and SIO output transitions.
- Not modeled or claimed: CYW43439 Wi-Fi/Bluetooth, its radio bus and firmware,
  the wireless-driven onboard LED, USB CDC/UF2 boot, and other RP2040
  peripherals outside this smoke profile.
- The board profile has no routed power net; the engine assumes an ideal VDD
  rail, so power-on and brown-out reset behavior is not simulated.

See [`evidence/vendor-status.md`](evidence/vendor-status.md) for the live
availability check and [`REQUIRED_DOCS.md`](REQUIRED_DOCS.md) for source links.
