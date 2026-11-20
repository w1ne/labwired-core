# Live vendor status — Arduino Nano 33 IoT + Adafruit BME280 breakout

Checked live on 2026-11-19 UTC (20:55 to 21:00).

- Microchip, required live Chrome check:
  `~/.venvs/mesh/bin/python ~/.hermes/scripts/mc-live-status.py ATSAMD21G18`
  → `ATSAMD21G18      In Production`.
  Vendor page: https://www.microchip.com/en-us/product/ATSAMD21G18
- Arduino store, https://store.arduino.cc/products/arduino-nano-33-iot, read
  in Chrome (the page answers HTTP 429 to curl) → `Arduino Nano 33 IoT`,
  `SKU ABX00027`, `ADD TO CART`, price EUR 18,70 (list EUR 26,70).
- Adafruit BME280 breakout, https://www.adafruit.com/product/2652: last read
  on 2026-11-19 15:58 UTC for `examples/arduino-nano-every-bme280` →
  schema.org availability `InStock`, price 14.95 (USD). This shows a buyable
  assembled module that carries the BME280. It does not claim bare-sensor
  stock at any distributor.

Not verified, and not modelled: the on-board IMU. The ST page
https://www.st.com/en/mems-and-sensors/lsm6ds3.html redirects to the MEMS
category page, so the LSM6DS3 status could not be read. The LSM6DS3TR-C page
shows `ACTIVE`, but that is a different part. No IMU model was added.
