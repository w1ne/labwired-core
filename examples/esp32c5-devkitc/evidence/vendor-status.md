# Live vendor availability — ESP32-C5-DevKitC-1-N8R8

Board availability rechecked live on 2026-11-10; sensor listings were checked
on 2026-11-06:

- Espressif's [ESP32-C5-DevKitC-1 v1.2 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c5/esp32-c5-devkitc-1/user_guide.html)
  lists the current `ESP32-C5-DevKitC-1-N8R8` variant, identifies its
  ESP32-C5-WROOM-1 module, 8 MB flash and 8 MB PSRAM, and provides retail and
  wholesale order paths.
- DigiKey's [exact N8R8 listing](https://www.digikey.com/en/products/detail/espressif-systems/ESP32-C5-DEVKITC-1-N8R8/28718162)
  showed manufacturer **Espressif Systems**, **Part Status: Active**,
  **312 in stock**, and **US$15.00** for one unit. This is live distributor
  evidence that the board is currently purchasable; inventory and pricing can
  change.

- Adafruit's [BME280 breakout PID 2652](https://www.adafruit.com/product/2652)
  page offered the sensor for purchase. DigiKey's [matching PID 2652 listing](https://www.digikey.com/en/products/detail/adafruit-industries-llc/2652/5604372)
  showed **Part Status: Active** and **1,990 in stock** on 2026-11-06.

The model targets the current v1.2 board guide. It does not claim the older
v1.1 board revision or the C5 v0.1 silicon described in the older guide.

The current board guide identifies the addressable RGB LED on GPIO27 and the
J1 header locations for GPIO6 and GPIO10. The board smoke exercises those I2C
pads with a BME280 model and drives the LED through RMT TX0 and GPIO27. Source:
[ESP32-C5-DevKitC-1 v1.2 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c5/esp32-c5-devkitc-1/user_guide.html).
