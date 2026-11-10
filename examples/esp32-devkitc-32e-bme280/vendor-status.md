# Live vendor and buyability check

Checked 2026-11-10 at 21:36 UTC.

- Espressif's current [ESP32-DevKitC V4 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32/esp32-devkitc/user_guide.html)
  lists the exact `ESP32-DevKitC-32E` board with an `ESP32-WROOM-32E` module and
  a retail sample ordering path. The [current DevKitC product page](https://www.espressif.com/en/products/devkits/esp32-devkitc/)
  still lists the 32E board; neither page marks it NRND or obsolete.
- Mouser's live [exact-SKU listing](https://www.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-DevKitC-32E)
  identifies Espressif Systems as manufacturer and showed **1,678 in stock**
  for immediate shipment at **$10.21**.
- Adafruit's live [BME280 breakout listing](https://www.adafruit.com/product/2652)
  showed product 2652 **in stock** at **$14.95**.
- Espressif's guide maps GPIO22 to J3 pin 3 and GPIO21 to J3 pin 6. These are
  the selected I2C0 SCL/SDA pins; GPIO23 is J3 pin 2 and is the smoke output.

This records current listings and availability, not a physical purchase or
silicon test. Distributor stock is a snapshot from the check time.
