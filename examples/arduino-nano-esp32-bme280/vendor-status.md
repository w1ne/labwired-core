# Live vendor status — Arduino Nano ESP32 + BME280

Checked live on 2026-11-17 UTC; no archive snapshot was used.

- The [Arduino official store listing](https://store.arduino.cc/products/nano-esp32)
  identifies the Nano ESP32, SKU **ABX00092**, and displayed €14.26 with an
  **Add to cart** control. It identifies the module as u-blox NORA-W106
  (ESP32-S3) and lists I2C on A4/SDA and A5/SCL.
- Arduino's [official Nano ESP32 pin map](https://docs.arduino.cc/tutorials/nano-esp32/cheat-sheet/)
  maps A4 to GPIO11 and A5 to GPIO12. The [official full pinout](https://docs.arduino.cc/resources/pinouts/ABX00083-full-pinout.pdf)
  confirms the same route.
- Adafruit's [BME280 STEMMA QT product 2652](https://www.adafruit.com/product/2652)
  was listed at **$14.95**, **In stock**, with **Add to Cart**. Its standard
  I2C address for this profile is `0x77`; Adafruit's [BME280 pinout guide](https://learn.adafruit.com/adafruit-bme280-humidity-barometric-pressure-temperature-sensor-breakout/pinouts)
  says that is the default, with SDO left unconnected.

Demand position: ESP32 family rank **1** (38% of reported MCU use); Arduino
kits were used by 37% of survey respondents' main projects. This is additional
board/peripheral coverage on existing ESP32-S3 silicon, not a new part family.
