# Live vendor status: Adafruit ESP32 Feather V2 + BME280

Checked 2026-11-06 against Adafruit's current product pages and Espressif's
live ESP32 series datasheet.

- Adafruit's [ESP32 Feather V2 with Headers, PID 5900](https://www.adafruit.com/product/5900)
  is marked **In stock** and offers **Add to Cart** at $20.95. This is the
  electrically equivalent headered Feather V2 listing used for the board
  smoke. The PCB-antenna-only [PID 5400 listing](https://www.adafruit.com/product/5400)
  currently says **Out of stock**.
- The [BME280 STEMMA QT breakout, PID 2652](https://www.adafruit.com/product/2652)
  is marked **In stock** and offers **Add to Cart** at $14.95.
- The Feather's [official pinout guide](https://learn.adafruit.com/adafruit-esp32-feather-v2/pinouts)
  documents STEMMA QT SDA=GPIO22, SCL=GPIO20, and GPIO2 as the power pin that
  must be high before the connector works. GPIO2 also powers the board's
  NeoPixel; NeoPixel signaling is outside this profile.
- Espressif's [live ESP32 series datasheet, version 5.3](https://documentation.espressif.com/esp32_datasheet_en.html)
  lists ESP32-D0WD-V3 without an NRND/EOL label. The same table explicitly
  marks other variants NRND or EOL, so this evidence applies to the
  D0WD-V3-equipped Feather V2 revision only.

Stock and lifecycle pages are time-sensitive; recheck them before a later
onboarding round.
