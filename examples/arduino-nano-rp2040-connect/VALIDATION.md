# Validation — Arduino Nano RP2040 Connect

The smoke builds bare-metal RP2040 firmware and checks that header TX / GP0
changes state through the modeled IO_BANK0 and SIO blocks. The NINA-W102,
onboard sensors and other omitted parts are outside scope; see
[`KNOWN_LIMITATIONS.md`](KNOWN_LIMITATIONS.md).

Run the command in [`README.md`](README.md) from the repository root. The
build logs, local CLI smoke result and GPIO edge evidence are committed in
[`evidence/`](evidence/).

The hosted verifier could not verify this board-specific Rust image; its
fallback attempts and exact outcomes are recorded in
[`evidence/labwired-verification.md`](evidence/labwired-verification.md).
