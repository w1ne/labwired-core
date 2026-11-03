# Validation — Adafruit Feather RP2040

The board's narrow system profile uses the RP2040's IO_BANK0 and SIO models to
toggle the built-in D13 LED on GP13. It does not claim the board's other
onboard devices or peripherals; see [`KNOWN_LIMITATIONS.md`](KNOWN_LIMITATIONS.md).

Reproduce the smoke from the repository root with the command in
[`README.md`](README.md). The generated build logs, CLI result and asserted
GPIO edge evidence are committed in [`evidence/`](evidence/).
