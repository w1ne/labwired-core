# Validation — Adafruit ItsyBitsy RP2040

The focused profile uses the RP2040 IO_BANK0 and SIO models to toggle the
manufacturer-mapped red D13 LED on GP11. The smoke asserts at least eight
transitions on `sio:11`.

Reproduce it with the command in [`README.md`](README.md). Build logs, the
simulator result, and GPIO edge evidence are committed in [`evidence/`](evidence/).
