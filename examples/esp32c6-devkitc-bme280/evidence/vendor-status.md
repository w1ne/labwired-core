# Live vendor availability — ESP32-C6-DevKitC-1-N8 + Adafruit BME280

Checked live on 2026-11-05:

- Espressif's [ESP32-C6-DevKitC-1 v1.2 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c6/esp32-c6-devkitc-1/user_guide.html)
  documents the current DevKitC-1 board and its N8 configuration.
- DigiKey's [ESP32-C6-DEVKITC-1-N8 listing](https://www.digikey.com/en/products/detail/espressif-systems/ESP32-C6-DEVKITC-1-N8/17728861)
  showed **Part Status: Active**, **289 in stock**, and a listed unit price of
  **$9.00**.
- DigiKey's [Adafruit PID 2652 listing](https://www.digikey.com/en/products/detail/adafruit-industries-llc/2652/5604372)
  showed **Part Status: Active**, **2,343 in stock**, and a listed unit price
  of **$14.95**.
- Adafruit's [BME280 breakout guide](https://learn.adafruit.com/adafruit-bme280-humidity-barometric-pressure-temperature-sensor-breakout)
  says the breakout's default I2C address is `0x77`; joining SDO to GND changes
  it to `0x76`.

Availability and prices are time-sensitive. The smoke configuration matches
the breakout's default address. This is vendor catalog evidence, not a
physical test of either item.
