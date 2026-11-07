# Adafruit Feather RP2040

This profile adds Adafruit's Feather RP2040 (product 4884) as a board-bound
system. The smoke builds bare-metal RP2040 firmware, selects SIO on GP13, and
toggles the board's built-in D13 red LED through IO_BANK0 and SIO.

The board profile is intentionally limited to that working GPIO path. The
RP2040 descriptor records the Feather's 8 MiB QSPI flash and 264 KiB SRAM; it
does not claim that the remaining board peripherals are modeled. See
[`KNOWN_LIMITATIONS.md`](KNOWN_LIMITATIONS.md) and the live stock check in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

Run from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id adafruit-feather-rp2040 \
  --crate firmware-feather-rp2040-smoke \
  --target thumbv6m-none-eabi \
  --script examples/adafruit-feather-rp2040/io-smoke.yaml \
  --system configs/systems/adafruit-feather-rp2040.yaml \
  --out-dir examples/adafruit-feather-rp2040/evidence
```

The committed logs and GPIO edge assertion are under [`evidence/`](evidence/).
