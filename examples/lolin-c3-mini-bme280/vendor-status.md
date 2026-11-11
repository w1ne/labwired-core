# Live vendor status

Checked 2026-11-11 UTC.

- **WEMOS LOLIN C3 Mini V2.1.0:** the current [WEMOS C3 Mini page](https://www.wemos.cc/en/latest/c3/c3_mini.html)
  calls V2.1.0 the latest version, identifies ESP32-C3FH4, and provides a
  **Buy it** link to the vendor's AliExpress listing. The vendor page did not
  expose a stock count in the documentation view. The current [HobbyElectronica
  listing](https://www.hobbyelectronica.nl/en/product/lolin-c3-mini/) for the
  same V2.1.0 board showed **47 in stock** at €9.95.
- **ESP32-C3FH4:** Espressif's [current ESP32-C3 datasheet](https://documentation.espressif.com/esp32-c3_datasheet_en.html)
  lists C3FH4 without an EOL or NRND marker. The same table explicitly labels
  C3FN4 as EOL and C3FH4AZ as NRND; this board is based on C3FH4, not either of
  those variants.
- The [official V2.1.0 schematic](https://www.wemos.cc/en/latest/_static/files/sch_c3_mini_v2.1.0.pdf)
  places GPIO4, GPIO5, and GPIO6 on the board headers and connects the onboard
  WS2812B data input to GPIO7.
- The [Adafruit BME280 breakout](https://www.adafruit.com/product/2652) is the
  external sensor represented in the system descriptor. Its live product page
  listed it at $14.95 with an **Add to Cart** control.

These are vendor product and status records, not a physical purchase or board
test.
