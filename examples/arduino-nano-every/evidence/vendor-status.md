# Vendor status evidence

Checked 2026-11-06 with the required live Microchip Chrome check:

- Part: ATMEGA4809
- Result: In Production
- Vendor URL: https://www.microchip.com/en-us/product/ATMEGA4809
- Command: `~/.venvs/mesh/bin/python ~/.hermes/scripts/mc-live-status.py ATMEGA4809`

Command output:
  ATMEGA4809       In Production

Re-checked 2026-11-08 for the Arduino matrix row:

- `~/.venvs/mesh/bin/python ~/.hermes/scripts/mc-live-status.py ATMEGA4809`
  → `ATMEGA4809       In Production`
- https://store.arduino.cc/products/nano-every.js → `Arduino Nano Every`,
  `available: true`, variant `ABX00028` available, price 1450 (EUR 14.50).

Re-checked 2026-11-08 for the copyable Arduino quickstart:

- `~/.venvs/mesh/bin/python ~/.hermes/scripts/mc-live-status.py ATMEGA4809`
  → `ATMEGA4809       In Production`
- https://store.arduino.cc/products/nano-every → product ABX00028 at €14.50
  with an Add to cart option.
