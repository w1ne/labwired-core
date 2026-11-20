# Live vendor status — Arduino Nano Every + Adafruit BME280 breakout

Checked live on 2026-11-19 UTC (15:58).

- Microchip, required live Chrome check:
  `~/.venvs/mesh/bin/python ~/.hermes/scripts/mc-live-status.py ATMEGA4809`
  → `ATMEGA4809       In Production`.
  Vendor page: https://www.microchip.com/en-us/product/ATMEGA4809
- Arduino store, https://store.arduino.cc/products/nano-every.js →
  `Arduino Nano Every`, `available: true`, variant `ABX00028` available,
  price 1450 (EUR 14.50).
- Adafruit BME280 breakout, https://www.adafruit.com/product/2652 → schema.org
  availability `InStock`, price 14.95 (USD). This shows a buyable assembled
  module that carries the BME280. It does not claim bare-sensor stock at any
  distributor.

The ATmega328P on the classic Nano is Not Recommended for New Designs
(Microchip live check 2026-11-05, `examples/arduino-uno-blinky/evidence/vendor-status.md`).
The Nano Every is the in-production Arduino board in the same form factor.
