# Validation — Raspberry Pi Pico 2 W

The narrow `raspberry-pi-pico-2-w` system boots bare-metal RP2350 firmware in
the local CLI smoke and checks that GP0 changes state through the modeled
IO_BANK0 and SIO blocks.
Wi-Fi, Bluetooth, USB, and the wireless-controlled onboard LED are outside the
descriptor; see [`KNOWN_LIMITATIONS.md`](KNOWN_LIMITATIONS.md).

Run the command in [`README.md`](README.md) from the repository root. The
committed `evidence/` directory contains the build and CLI smoke result. The
hosted LabWired verifier has no Pico 2 W target or Rust toolchain; its attempted
RP2350-Zero fallback returned `unsupported`. See
[`evidence/labwired-verification.md`](evidence/labwired-verification.md).
