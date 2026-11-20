# Live vendor status — ESP32-DevKitM-1 + BME280

Checked live on 2026-11-17 UTC.

- Espressif's [ESP32-DevKitM-1 user guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32/esp32-devkitm-1/user_guide.html)
  identifies the exact board as an ESP32-MINI-1 design with 4 MB flash and
  documents its header pins. GPIO21 and GPIO22 are exposed, so the BME280 can
  be wired to those pins without changing the board.
- Espressif's [current DevKits catalog](https://www.espressif.com/en/products/devkits/esp32-devkitm-1/overview)
  lists the exact ESP32-DevKitM-1 and links to DigiKey, Mouser, Amazon US, and
  Robu India for ordering.
- Mouser's [ESP32-DEVKITM-1 listing](https://www.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-DEVKITM-1)
  showed **124 in stock**, **Can Ship Immediately**, and **$8.17** (quantity 1)
  when checked. Inventory and price can change.
- Adafruit's [BME280 STEMMA QT listing](https://www.adafruit.com/product/2652)
  showed **In stock** at **$14.95** when checked. The sensor is attached at
  I2C address `0x77`; Adafruit's [BME280 pinout guide](https://learn.adafruit.com/adafruit-bme280-humidity-barometric-pressure-temperature-sensor-breakout/pinouts)
  documents the I2C pins and default address.

The board and breakout are both currently orderable. Espressif does not put an
Active/NRND lifecycle label on the DevKitM-1 catalog entry; the evidence here
is the live vendor listing plus current distributor stock.
