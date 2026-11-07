# Adafruit Feather RP2350 with HSTX

| Item | Value |
|---|---|
| Board | Adafruit Feather RP2350 with HSTX, product #6000, no-PSRAM variant |
| MCU | Raspberry Pi RP2350A, Cortex-M33 profile |
| System YAML | [`configs/systems/adafruit-feather-rp2350.yaml`](../../configs/systems/adafruit-feather-rp2350.yaml) |
| Chip profile | [`configs/chips/rp2350-feather-smoke.yaml`](../../configs/chips/rp2350-feather-smoke.yaml) |
| Smoke | [`examples/adafruit-feather-rp2350/`](../../examples/adafruit-feather-rp2350/README.md) |

## Exercised path

The narrow profile maps Adafruit's active-high red indicator LED to GPIO7.
Firmware configures the RP2350 IO_BANK0 function select for SIO and toggles
the pin. The smoke requires at least eight GPIO edges.

## Scope

The descriptor includes 8 MB flash geometry and 520 KB SRAM, IO_BANK0, SIO,
and the board's GPIO7 LED wiring. It does not claim the board's status
NeoPixel/PIO path (GPIO21), STEMMA QT I2C (GPIO2/GPIO3), USB, HSTX, optional
PSRAM, or other header pins. The flash geometry does not claim QSPI controller
behavior. The RP2350's second core and Hazard3 execution path are also outside
this board smoke.

The board was listed in stock by Adafruit on 2026-11-04; the recorded live
check and source URLs are in the example's `evidence/vendor-status.md`. This is
a simulator smoke only, not physical hardware confirmation.
