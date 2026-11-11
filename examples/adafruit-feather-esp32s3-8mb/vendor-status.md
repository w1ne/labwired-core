# Vendor status

Checked 2026-11-11. Adafruit Product 5323 lists the **8 MB Flash, No PSRAM**
variant as in stock and purchasable. The separate 4 MB Flash + 2 MB PSRAM
variant is not the target of this descriptor.

- Product and current availability: [Adafruit ESP32-S3 Feather, Product 5323](https://www.adafruit.com/product/5323)
- Board pinout and built-in NeoPixel: [Adafruit ESP32-S3 Feather pinouts](https://learn.adafruit.com/adafruit-esp32-s3-feather/pinouts)
- Official Arduino variant pin map: [Espressif `adafruit_feather_esp32s3_nopsram` pins](https://github.com/espressif/arduino-esp32/blob/master/variants/adafruit_feather_esp32s3_nopsram/pins_arduino.h)

The official variant assigns the NeoPixel data line to GPIO33 and its power
enable to GPIO21. The profile models UART0 and the GPIO33 NeoPixel data stream.
It does not claim the battery charger/monitor, STEMMA QT power switch, USB,
Wi-Fi, or BLE.
