# Live availability — Arduino Nano ESP32 and Adafruit BME280

Checked live on 2026-11-17; no Wayback snapshots used.

- DigiKey's [Arduino ABX00092 product page](https://www.digikey.co.uk/en/products/detail/arduino/ABX00092/21219771)
  listed the board as **Active** with **1,385 in stock**. The board is the
  Nano ESP32 with ESP32-S3 / NORA-W106-10B.
- Arduino's [Nano ESP32 pinout](https://docs.arduino.cc/tutorials/nano-esp32/cheat-sheet/)
  maps A4/SDA to GPIO11 and A5/SCL to GPIO12. These are the pins in the board
  system descriptor and this smoke.
- Adafruit's [BME280 STEMMA QT breakout, product 2652](https://www.adafruit.com/product/2652)
  listed **In stock** at USD 14.95 with an Add to Cart control. Its product
  page describes I²C support and the STEMMA QT connectors.
- Bosch's current [BME280 product page](https://www.bosch-sensortec.com/en/products/environmental-sensors/humidity-sensors-bme280)
  describes a current sensor and states a **10 years delivery** commitment.

The model claims only the BME280's I²C identity response (0x60) and this board's
A4/A5 wiring. It does not claim physical measurements or board bring-up.

Bosch's page showed zero stock at its listed distributors during this recheck.
The buyability evidence for this specific design is Adafruit's in-stock
product 2652 breakout above.
