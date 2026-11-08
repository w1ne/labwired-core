# Adafruit ItsyBitsy RP2040 — D13 GPIO smoke

The board profile maps the onboard red D13 LED to RP2040 GP11 and exercises the
IO_BANK0 and SIO models with repeated GPIO transitions.

## Status at a glance

| Aspect | Status |
|---|---|
| Chip yaml | [`configs/chips/rp2040-itsybitsy-smoke.yaml`](../../configs/chips/rp2040-itsybitsy-smoke.yaml) |
| System yaml | [`configs/systems/adafruit-itsybitsy-rp2040.yaml`](../../configs/systems/adafruit-itsybitsy-rp2040.yaml) |
| Smoke crate | `firmware-itsybitsy-rp2040-smoke` |
| Evidence | [`examples/adafruit-itsybitsy-rp2040/evidence/`](../../examples/adafruit-itsybitsy-rp2040/evidence/) |
| Tier | **smoke-manual** — local GPIO model evidence; no silicon capture |

## What the smoke exercises

The firmware selects SIO for GP11, enables the pin as an output, and repeatedly
sets and clears it. Adafruit's pinout guide maps D13 and the red LED to GP11.
The board-specific descriptor reflects 8 MiB of external flash and 264 KiB of
SRAM, but only declares the two peripherals used by the smoke.

## Scope limits

The profile omits the NeoPixel, its power control, buttons, USB/UF2, QSPI boot,
UART, SPI, I2C, ADC, PWM, DMA, and other board features. No zero-answering
peripheral stubs are loaded. The simulator assumes ideal VDD, so power-on and
brown-out reset behavior are not covered. This is local model evidence, not a
physical-board confirmation.

## Vendor status

Adafruit's product page showed 46 in stock at $9.95 with an **Add to Cart**
control during the 2026-11-04 live check. Source and pinout URLs are recorded in
[`examples/adafruit-itsybitsy-rp2040/evidence/vendor-status.md`](../../examples/adafruit-itsybitsy-rp2040/evidence/vendor-status.md).

## Reproduce

See [`examples/adafruit-itsybitsy-rp2040/README.md`](../../examples/adafruit-itsybitsy-rp2040/README.md)
for the smoke command and acceptance criteria.
