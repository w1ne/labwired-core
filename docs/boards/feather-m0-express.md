# Adafruit Feather M0 Express

The Feather M0 Express uses Microchip's ATSAMD21G18A. Its focused LabWired
profile boots bare-metal firmware and toggles the board's red D13 LED on PA17
through the modeled SAMD21 PORT block. The smoke enables the PORT APB clock
and asserts 16 GPIO edges.

Run it with:

```bash
scripts/onboarding_smoke.sh \
  --target-id feather-m0-express \
  --crate firmware-feather-m0-express-smoke \
  --target thumbv6m-none-eabi \
  --script examples/feather-m0-express/io-smoke.yaml \
  --system configs/systems/feather-m0-express.yaml \
  --out-dir examples/feather-m0-express/evidence
```

The board was listed in stock and the MCU in production on 2026-11-04; see
[`the vendor record`](../../examples/feather-m0-express/evidence/vendor-status.md).
The profile omits USB, the NeoPixel, external flash, SERCOM buses, ADC/DAC,
timers, and the charger. The smoke stays at the SAMD21's 1 MHz reset clock.
