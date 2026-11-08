# Validation — Raspberry Pi Pico W

The smoke builds bare-metal firmware for the `raspberry-pi-pico-w` system and
checks that GP0 changes state through the modeled IO_BANK0 and SIO blocks.
Wireless and the wireless-driven onboard LED are out of scope; see
[`KNOWN_LIMITATIONS.md`](KNOWN_LIMITATIONS.md).

Run the command in [`README.md`](README.md) from the repository root. The
build logs, CLI smoke result, and GPIO edge evidence are committed in
[`evidence/`](evidence/).
