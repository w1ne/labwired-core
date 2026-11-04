# Adafruit Feather RP2350 with HSTX

This board profile boots the RP2350 and toggles Adafruit's onboard red
indicator LED on GPIO7. The board is Adafruit product #6000, the 8 MB flash
variant without the optional PSRAM fitted.

The descriptor is intentionally limited to the exercised IO_BANK0 pad route
and SIO GPIO path. The status NeoPixel on GPIO21, STEMMA QT I2C on GPIO2/3,
USB, HSTX, optional PSRAM, and remaining header pins are not modeled by this
profile. The 8 MB flash value describes the board's memory geometry; QSPI
controller behavior is not part of this smoke.

Run the committed onboarding smoke from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id adafruit-feather-rp2350 \
  --crate firmware-adafruit-feather-rp2350-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/adafruit-feather-rp2350/io-smoke.yaml \
  --system configs/systems/adafruit-feather-rp2350.yaml \
  --out-dir examples/adafruit-feather-rp2350/evidence \
  --firmware-build-dir crates/firmware-adafruit-feather-rp2350-smoke
```

Build and simulator logs, the GPIO edge assertion, and current vendor
availability evidence are committed under [`evidence/`](evidence/). This is a
local deterministic model smoke, not physical-hardware confirmation.
