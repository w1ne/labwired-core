# Live availability and board pin evidence

Checked live on 2026-11-09 for the ESP32-C61-DevKitC-1-N8R2 plus the modeled
Adafruit BME280 breakout:

- Espressif's [ESP32-C61-DevKitC-1 v2.0 guide](https://documentation.espressif.com/esp-dev-kits/en/latest/esp32c61/esp32-c61-devkitc-1/index.html)
  lists the exact `ESP32-C61-DevKitC-1-N8R2` ordering variant, says retail
  samples can be ordered, maps J1 pin 3 to GPIO4 and J1 pin 4 to GPIO5, and
  identifies the onboard RGB LED as GPIO8.
- DigiKey's [exact N8R2 listing](https://www.digikey.com/en/products/detail/espressif-systems/ESP32-C61-DEVKITC-1-N8R2/26763163)
  returned **Part Status: Active**, **217 in stock**, and a one-unit price of
  **US$9.00**. Stock and pricing can change.
- DigiKey's [Adafruit 2652 BME280 breakout listing](https://www.digikey.com/en/products/detail/adafruit-industries-llc/2652/5604372)
  returned **Part Status: Active**, I2C/SPI interface, **3,466 in stock**, and a
  one-unit price of **US$14.95**. This is the concrete buyable sensor board
  assumed by the system profile. Bosch's live [BME280 product page](https://www.bosch-sensortec.com/en/products/environmental-sensors/humidity-sensors-bme280)
  displayed zero stock at its listed distributors during this check, so this
  round relies on the active Adafruit breakout listing rather than claiming
  bare-sensor stock from Bosch.

The system uses the board's 3V3/GND headers and connects the breakout SDA/SCL
to GPIO4/GPIO5 at J1 pins 3/4, with the model configured for I2C address `0x77`.

## Register-map source

The [ESP32-C61 Technical Reference Manual](https://documentation.espressif.com/esp32-c61_technical_reference_manual_en.pdf)
is the source for the C61 I2C0/PCR/GPIO matrix register locations used by this
profile; the C61 signal indices route I2C0 SCL/SDA through matrix signals 46/47.
