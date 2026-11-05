# Live vendor and availability record — ESP32-DevKitC-32E

Checked live on 2026-11-05:

- Espressif's [ESP32-DevKitC V4 guide](https://documentation.espressif.com/esp-dev-kits/en/latest/esp32/esp32-devkitc/user_guide.html)
  lists exact orderable variant **ESP32-DevKitC-32E**, with ESP32-WROOM-32E,
  4 MB flash, and no PSRAM. The live guide maps GPIO23 to J3 pin 2 and provides
  a retail sample ordering path. The same guide marks ESP32-DevKitC V2 as the
  older version; this smoke targets V4 / 32E.
- DigiKey's [live ESP32-DevKitC-32E product listing](https://www.digikey.com/en/products/detail/espressif-systems/ESP32-DEVKITC-32E/12091810)
  lists manufacturer part **ESP32-DEVKITC-32E**, product status **Active**,
  currently 0 in stock and 252 expected on 2027-07-29. This current lookup
  does not show DigiKey stock available today.
- Mouser's [ESP32-DevKitC-32E listing](https://www.mouser.com/ProductDetail/Espressif-Systems/ESP32-DevKitC-32E?qs=GedFDFLaBXFpgD0kAZWDrQ%3D%3D)
  live search result reported **972 in stock** and **can ship immediately**
  when checked earlier on 2026-11-05. Counts are time-sensitive. The Mouser
  result is the same-day purchase evidence for this extension; DigiKey's
  current direct listing is Active but out of stock.

The target is the current WROOM-32E module variant, not an older ESP32-WROOM-32D
or ESP32-DevKitC V2. The smoke covers GPIO23 and UART0 console output. The
USB-to-UART bridge, pin-level UART waveform, Wi-Fi/Bluetooth, reset circuitry,
and power LED are not claimed.
