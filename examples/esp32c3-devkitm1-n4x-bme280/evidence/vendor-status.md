# Live vendor availability — ESP32-C3-DevKitM-1-N4X + BME280

Checked live on 2026-11-06:

- Espressif's [current ESP32-C3-DevKitM-1 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c3/esp32-c3-devkitm-1/user_guide.html)
  lists the exact **ESP32-C3-DevKitM-1-N4X** 4 MB SKU and marks the older
  unsuffixed variant **EOL**. It documents GPIO4/GPIO5 on the J3 header and
  points retail orders to Espressif's sample channel.
- DigiKey's [exact N4X listing](https://www.digikey.com/en/products/detail/espressif-systems/esp32-c3-devkitm-1-n4x/25587617)
  reported **Part Status: Active**, **692 in stock**, and an **Add to Cart**
  option at the time checked.
- Adafruit's [BME280 STEMMA QT breakout, PID 2652](https://www.adafruit.com/product/2652)
  was listed **In stock** with an **Add to Cart** option. The product page
  describes its I2C interface and board-level voltage regulation/level
  shifting; the breakout's default address is `0x77`.
- Bosch's [BME280 product page](https://www.bosch-sensortec.com/en/products/environmental-sensors/humidity-sensors-bme280)
  lists I2C/SPI interfaces and a 10-year delivery commitment.

The profile matches the exact active N4X board SKU and the Adafruit breakout's
default I2C address. These listings establish current catalog availability,
not physical validation of the components.
