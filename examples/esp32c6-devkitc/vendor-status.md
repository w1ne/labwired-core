# Live availability — ESP32-C6-DevKitC-1-N8 + BME280

Checked live on **2026-11-11 UTC**.

- Espressif's [development-kit catalog](https://www.espressif.com/products/devkits)
  still lists the ESP32-C6-DevKitC-1-N8 and links to distributors. DigiKey's
  [exact-part listing](https://www.digikey.com/en/products/detail/espressif-systems/ESP32-C6-DEVKITC-1-N8/17728861)
  showed the exact board **Active** with **285 in stock** when checked. Counts
  change; this is a dated stock observation, not a future guarantee.
- Bosch's [BME280 product page](https://www.bosch-sensortec.com/en/products/environmental-sensors/humidity-sensors-bme280)
  remains live and lists I²C/SPI plus a 10-year delivery statement. DigiKey's
  [BME280 listing](https://www.digikey.com/en/products/detail/bosch-sensortec/BME280/6136306)
  labels the part **Active**. Adafruit's [assembled STEMMA QT breakout, PID
  2652](https://www.adafruit.com/product/2652) showed **$14.95** and **In
  stock**, the exact module used in the example.
- Espressif's [ESP32-C6-DevKitC-1 v1.2 user guide](https://documentation.espressif.com/esp-dev-kits/en/latest/esp32c6/esp32-c6-devkitc-1/user_guide.html)
  lists GPIO6 and GPIO7 on header J1. The smoke uses GPIO6 as SDA and GPIO7 as
  SCL through the C6 GPIO matrix; the board's USB-to-UART pins GPIO16/17 are
  left free.

This checks current vendor listings and distributor availability. The model
does not verify the physical board or breakout.
