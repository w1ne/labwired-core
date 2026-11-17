# Live vendor status — Arduino Nano ESP32

Checked live on 2026-11-11 against Arduino's official store. The [official
store product page](https://store.arduino.cc/products/nano-esp32) listed the
Arduino Nano ESP32 (SKU ABX00092) at €14.26 (sale price, €20.40 listed price)
with an **Add to cart** control. The [official board page](https://docs.arduino.cc/hardware/nano-esp32)
identifies the NORA-W106 / ESP32-S3 module and 16 MB flash. The [official store
specification](https://store.arduino.cc/products/nano-esp32) lists built-in LED
pin 13 and separate RGB LED pins 14–16. The [official
pinout](https://docs.arduino.cc/resources/pinouts/ABX00083-full-pinout.pdf)
maps D13 / `LED_BUILTIN` to GPIO48, A4/SDA to GPIO11, A5/SCL to GPIO12, the
separate RGB channels to GPIO46/GPIO0/GPIO45, and USB D−/D+ to GPIO19/GPIO20.

The current store item is SKU ABX00092; the board documentation and schematic
also identify the Nano ESP32 as ABX00083. This smoke covers the shared Nano
ESP32 GPIO48 behavior only. It is not a silicon test.

## Recheck 2026-11-16 (stock Arduino matrix row)

Checked live on 2026-11-16. No Wayback snapshot was used.

- [DigiKey ABX00092](https://www.digikey.com/en/products/detail/arduino/ABX00092/21219771)
  lists "ARDUINO NANO ESP32 NO HEADERS", part status "Active", 1,061 in stock,
  USD 18.30 at quantity 1. The Arduino store returned HTTP 429 to every
  request on this date, so it was not reread.
- The [official board page](https://docs.arduino.cc/hardware/nano-esp32/) has
  a "Buy now" link and no retired or end-of-life notice.
- The [Espressif longevity commitment](https://www.espressif.com/en/products/longevity-commitment)
  lists ESP32-S3 with "At Least 12 Years" from 2021-01-01, with no NRND or EOL
  mark.
