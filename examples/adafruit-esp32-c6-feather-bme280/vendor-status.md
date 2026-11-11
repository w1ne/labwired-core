# Live vendor status

Checked 2026-11-11 14:35 UTC.

- **Adafruit ESP32-C6 Feather (PID 5933):** Adafruit's live [product page](https://www.adafruit.com/product/5933) identifies the ESP32-C6 Feather and its STEMMA QT connector; its store page showed out of stock. DigiKey's live [exact PID 5933 listing](https://www.digikey.com/en/products/detail/adafruit-industries-llc/5933/24892281) identified the utilized IC as ESP32-C6, marked the item **Active**, and showed **63 in stock** at $14.95 when checked. The open [Adafruit PCB repository](https://github.com/adafruit/Adafruit-ESP32-C6-Feather-PCB) includes the board schematic, which names an ESP32-C6-MINI module.
- **ESP32-C6 silicon:** Adafruit's current [pinout guide](https://learn.adafruit.com/adafruit-esp32-c6-feather/pinouts) describes a 160 MHz C6 module with 4 MB flash. Espressif's current [ESP32-C6-MINI-1 module datasheet](https://documentation.espressif.com/esp32-c6-mini-1_mini-1u_datasheet_en.html) identifies the C6FH4/C6FH8 dies used in the MINI module family and the 4 MB C6FH4 configurations. Espressif's live [ESP32-C6 series datasheet](https://documentation.espressif.com/esp32-c6_datasheet_en.html), version 1.6 (2026-11-10), includes ESP32-C6FH4 and has no NRND/EOL marking. DigiKey's [ESP32-C6-MINI-1-H4 listing](https://www.digikey.com/en/products/detail/espressif-systems/ESP32-C6-MINI-1-H4/17885794) also marks that module **Active** and showed stock.
- **Adafruit BME280 STEMMA QT breakout (product 2652):** Adafruit's live [product listing](https://www.adafruit.com/product/2652) identifies the I²C/SPI breakout and showed **In stock** at $14.95 when checked. Bosch's current [BME280 product page](https://www.bosch-sensortec.com/en/products/environmental-sensors/humidity-sensors-bme280) lists the sensor and current datasheet.

These are live catalog checks, not a purchase or physical-board test. Board
stock can change; the distributor listing is the buyability evidence because
Adafruit's own product page was out of stock at check time.
