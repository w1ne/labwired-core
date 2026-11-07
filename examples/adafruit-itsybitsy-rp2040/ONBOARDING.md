# Adafruit ItsyBitsy RP2040 onboarding

## Board and live status

Adafruit's live product page showed 46 in stock at $9.95 with an **Add to Cart**
control on 2026-11-04. The official pinout guide maps D13 and the red user LED
to RP2040 GP11. The URLs and exact observations are in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Modeled board scope

[`configs/systems/adafruit-itsybitsy-rp2040.yaml`](../../configs/systems/adafruit-itsybitsy-rp2040.yaml)
binds the board to a focused RP2040 descriptor with its 8 MiB flash, 264 KiB
SRAM, GP11 pin map, and IO_BANK0/SIO models. Firmware selects the SIO function
on GP11 and toggles the output connected to the onboard D13 LED.

Only this GPIO path is modeled. The NeoPixel, buttons, USB/UF2, UART, SPI, I2C,
ADC, PWM, DMA, and physical QSPI boot path are outside the profile; no zero
windows are installed for those devices.

## Smoke evidence

The committed local smoke requires at least eight transitions on `sio:11` and
stops at the configured step limit. Reproduction instructions and generated
evidence are in [`README.md`](README.md) and [`evidence/`](evidence/). This is
simulator evidence, not a physical-board capture.
