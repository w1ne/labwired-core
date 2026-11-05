# Live vendor and availability record — ESP32-DevKitC-32E

Checked live on 2026-11-05:

- Espressif's [ESP32-DevKitC V4 guide](https://documentation.espressif.com/esp-dev-kits/en/latest/esp32/esp32-devkitc/user_guide.html)
  lists exact orderable variant **ESP32-DevKitC-32E**, with ESP32-WROOM-32E,
  4 MB flash, and no PSRAM. The live guide maps GPIO23 to J3 pin 2 and provides
  a retail sample ordering path. The same guide marks ESP32-DevKitC V2 as the
  older version; this smoke targets V4 / 32E.
- DigiKey's [live ESP32 DevKitC product listing](https://www.digikey.com/en/products/filter/evaluation-boards/rf-rfid-wireless-evaluation-boards/1165?s=N4Ig7CBcoIYE5QIwA5EGYA0IYBcmZAAcBLJAVjAE4AmZasAXwaA)
  lists manufacturer part **ESP32-DEVKITC-32E**, product status **Active**,
  currently 0 in stock and check lead time.
- Mouser's [ESP32-DevKitC-32E listing](https://www.mouser.com/ProductDetail/Espressif-Systems/ESP32-DevKitC-32E?qs=GedFDFLaBXFpgD0kAZWDrQ%3D%3D)
  search result reported **972 in stock** and **can ship immediately** when
  checked. Counts are time-sensitive; this records the availability evidence
  used for this round.

The target is the current WROOM-32E module variant, not an older ESP32-WROOM-32D
or ESP32-DevKitC V2. The smoke covers only GPIO23. Wi-Fi/Bluetooth, UART0, the
USB-to-UART bridge, reset circuitry, and power LED are not claimed.
