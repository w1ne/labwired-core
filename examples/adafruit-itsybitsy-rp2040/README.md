# Adafruit ItsyBitsy RP2040

This focused board profile exercises the red user LED on D13 / GP11. The
manufacturer pinout maps GP11 through IO_BANK0 to SIO, and the smoke requires at
least eight output transitions. The board-specific RP2040 descriptor records
the board's 8 MiB flash and 264 KiB SRAM while declaring only the IO_BANK0 and
SIO models needed by this smoke.

The NeoPixel, buttons, USB/UF2 and other peripheral paths are not claimed; see
[`KNOWN_LIMITATIONS.md`](KNOWN_LIMITATIONS.md). The live product availability
and pin mapping record is in [`evidence/vendor-status.md`](evidence/vendor-status.md).

Run from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id adafruit-itsybitsy-rp2040 \
  --crate firmware-itsybitsy-rp2040-smoke \
  --target thumbv6m-none-eabi \
  --script examples/adafruit-itsybitsy-rp2040/io-smoke.yaml \
  --system configs/systems/adafruit-itsybitsy-rp2040.yaml \
  --out-dir examples/adafruit-itsybitsy-rp2040/evidence
```

The generated logs and GPIO edge evidence are committed under [`evidence/`](evidence/).
