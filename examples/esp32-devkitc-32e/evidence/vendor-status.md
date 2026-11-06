# Live vendor lifecycle and availability — ESP32-DevKitC-32E + BME280

Checked 2026-11-06 against current Espressif documentation and distributor
listings.

- Espressif's [current ESP32-DevKitC V4 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32/esp32-devkitc/user_guide.html)
  identifies exact orderable part `ESP32-DevKitC-32E` as using the
  `ESP32-WROOM-32E` module, with 4 MB flash and no PSRAM. Its header table maps
  GPIO22 to J3 pin 3 and GPIO21 to J3 pin 6 and provides 3V3/GND on the headers.
- Mouser's [ESP32-DevKitC-32E listing](https://www.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-DevKitC-32E?mgh=1&qs=GedFDFLaBXFpgD0kAZWDrQ%3D%3D)
  showed **1,351 in stock**, **can ship immediately**, and **$10.20** in its
  current listing result. DigiKey's [same exact board listing](https://www.digikey.com/en/products/detail/espressif-systems/ESP32-DEVKITC-32E/12091810)
  labels the manufacturer part **Active** (DigiKey itself showed zero stock
  when checked).
- Espressif's [current ESP32-WROOM-32E / 32UE datasheet](https://documentation.espressif.com/esp32-wroom-32e_esp32-wroom-32ue_datasheet_en.html)
  v2.1 lists WROOM-32E-N4 4 MB flash variants and does not label the module
  NRND or EOL. Its silicon options include ESP32-D0WD-V3 or
  ESP32-D0WDR2-V3. The DevKitC-32E guide specifies a 4 MB, no-PSRAM board, so
  this smoke targets the D0WD-V3 configuration rather than the R2/PSRAM
  configuration. Espressif's [live ESP32 Series datasheet](https://documentation.espressif.com/esp32_datasheet_en.html)
  lists D0WD-V3 without NRND/EOL status and separately labels the old D0WDQ6
  and D0WDQ6-V3 variants NRND and D0WDR2-V3 EOL.
- DigiKey's exact [ESP32-WROOM-32E-N4 module listing](https://www.digikey.com/en/products/detail/espressif-systems/ESP32-WROOM-32E-N4/11613125)
  labels the module **Active** and identifies ESP32-D0WD-V3 as its utilized
  silicon. This corroborates the no-PSRAM board-level selection above.
- Adafruit's [BME280 I2C/SPI breakout, PID 2652](https://www.adafruit.com/product/2652)
  listed **$14.95**, **In stock**, and **Add to Cart**. The sensor is modeled
  at its standard I2C address `0x77`.

This round targets the exact active and stocked DevKitC-32E SKU and its
4 MB/no-PSRAM WROOM-32E configuration. The listing statements are
time-sensitive and establish purchase availability, not physical validation.
