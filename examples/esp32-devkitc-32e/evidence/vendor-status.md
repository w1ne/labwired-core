# Live vendor lifecycle and availability — ESP32-DevKitC-32E + RMT pixel smoke

Checked 2026-11-06 against current Espressif documentation and distributor
listings.

- Espressif's [current ESP32-DevKitC V4 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32/esp32-devkitc/user_guide.html)
  identifies exact orderable part `ESP32-DevKitC-32E` as using the
  `ESP32-WROOM-32E` module, with 4 MB flash and no PSRAM. Its header table maps
  GPIO22 to J3 pin 3 and GPIO21 to J3 pin 6 and provides 3V3/GND on the headers.
  The pin table also identifies GPIO23 on J3 pin 2, the external pixel data pin
  used by this RMT smoke.
- Mouser's [ESP32-DevKitC-32E listing](https://www.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-DevKitC-32E?mgh=1&qs=GedFDFLaBXFpgD0kAZWDrQ%3D%3D)
  reported the exact board **in stock**, **can ship immediately**, at **$10.20**.
  Its search-result variants showed different inventory counts (972–1,351), so
  this evidence records availability and omits a single count. DigiKey's [same
  exact board listing](https://www.digikey.com/en/products/detail/espressif-systems/ESP32-DEVKITC-32E/12091810)
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

- Espressif's [ESP32-WROOM technical-documents page](https://www.espressif.com/en/support/documents/technical-documents/esp32-wroom)
  lists the WROOM-32E/32UE datasheet v2.1 dated 2026-08-05. The current
  [WROOM-32E datasheet](https://documentation.espressif.com/esp32-wroom-32e_esp32-wroom-32ue_datasheet_en.html)
  does not mark the 32E module NRND or EOL. The exact DevKitC-32E purchase
  listing above showed stock and immediate shipment on the check date.

This round is an ESP32 rank-1 demand pick (38% measured MCU use) and targets
the exact active and stocked DevKitC-32E SKU and its 4 MB/no-PSRAM WROOM-32E
configuration. The attached WS2812 pixel is a generic external test fixture;
this work does not claim an onboard LED. Listing statements are time-sensitive
and establish purchase availability, not physical validation. The exact
current vendor sources and distributor availability were checked 2026-11-06.

The RMT register offsets, TX_START/TX_END fields, channel memory layout, GPIO
output signal index, and interrupt source use Espressif's classic ESP32
ESP-IDF headers: [`rmt_reg.h`](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32/register/soc/rmt_reg.h),
[`rmt_struct.h`](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32/register/soc/rmt_struct.h),
[`gpio_sig_map.h`](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32/include/soc/gpio_sig_map.h),
and [`interrupts.h`](https://github.com/espressif/esp-idf/blob/master/components/soc/esp32/include/soc/interrupts.h).
The local model's claim remains limited to channel 0 transmit behavior described
in the board README.
