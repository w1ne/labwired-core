# Live vendor status — Arduino Nano Every + Adafruit MAX31855 breakout

Checked live on 2026-11-19 UTC (17:25).

- Microchip, required live Chrome check:
  `~/.venvs/mesh/bin/python ~/.hermes/scripts/mc-live-status.py ATMEGA4809`
  → `ATMEGA4809       In Production`.
  Vendor page: https://www.microchip.com/en-us/product/ATMEGA4809
- Arduino store, https://store.arduino.cc/products/nano-every.js →
  `Arduino Nano Every`, `available: true`, variant `ABX00028` available,
  price 1450 (EUR 14.50).
- Adafruit MAX31855 breakout, https://www.adafruit.com/product/269 →
  schema.org availability `InStock`, price 14.95 (USD). This shows a buyable
  assembled module that carries the MAX31855.

Not read: the MAX31855 lifecycle status at Analog Devices.
https://www.analog.com/en/products/max31855.html answered `Access Denied` to
curl and to the real Chrome on 2026-11-19, and the DigiKey listing stopped at
a bot challenge. This change adds no MAX31855 model: the converter model
(`configs/devices/max31855.yaml`) was already in the repository.

The ATmega328P on the classic Nano is Not Recommended for New Designs
(Microchip live check 2026-11-05, `examples/arduino-uno-blinky/evidence/vendor-status.md`).
The Nano Every is the in-production Arduino board in the same form factor.
