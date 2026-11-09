# ESP32-DevKitC-32E lifecycle and availability check — ADC coverage

Checked 2026-11-09 UTC for the ADC coverage extension. This is the same exact
board SKU used by the existing DevKitC-32E examples.

- Espressif's current [ESP32-DevKitC V4 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32/esp32-devkitc/user_guide.html)
  identifies the ESP32-DevKitC-32E with an ESP32-WROOM-32E module and 4 MB
  flash. Its header table maps J2 pin 3 / VP to GPIO36 / ADC1_CH0.
- The exact [Mouser ESP32-DevKitC-32E listing](https://www.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-DevKitC-32E?mgh=1&qs=GedFDFLaBXFpgD0kAZWDrQ%3D%3D)
  lists the part at $10.20 with minimum quantity 1. The availability result
  returned during this check reported 1,351 in stock and immediate shipping.
- DigiKey's exact [ESP32-DEVKITC-32E listing](https://www.digikey.com/en/products/detail/espressif-systems/ESP32-DEVKITC-32E/12091810)
  marks the part **Active**. DigiKey's live detail page showed zero stock and
  an estimated replenishment date of 2027-08-09; a separate current DigiKey
  search result showed 3 units in stock. Inventory differed between live
  views, so the immediate-availability evidence is from Mouser.
- Espressif's live [ESP32-WROOM-32E datasheet](https://documentation.espressif.com/esp32-wroom-32e_esp32-wroom-32ue_datasheet_en.html)
  is version 2.1, dated 2026-08-05 on the vendor's
  [technical-documents page](https://www.espressif.com/en/support/documents/technical-documents/esp32-wroom),
  and does not mark WROOM-32E NRND or EOL. The DevKitC-32E guide specifies the
  4 MB, no-PSRAM configuration.

The potentiometer is an external simulation fixture used to drive ADC1_CH0;
it is not included on the board. These lifecycle and inventory statements are
time-sensitive. The board is listed as an active exact SKU and Mouser reported
immediate stock during this check.
