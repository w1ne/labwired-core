# Live vendor status — ESP32-S2-DevKitM-1-N4R2

Checked live on 2026-11-05:

- Espressif's [ESP32-S2-DevKitM-1 user guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32s2/esp32-s2-devkitm-1/user_guide.html)
  lists **ESP32-S2-DevKitM-1-N4R2** with **ESP32-S2-MINI-2 (Recommended)**,
  4 MB flash, and 2 MB PSRAM. The older MINI-1 board variants are explicitly
  marked EOL; this profile does not target them. The same guide says retail
  samples may be ordered and identifies the board's addressable RGB LED on
  GPIO18.
- Espressif's live [module catalog](https://www.espressif.com/products/modules)
  lists ESP32-S2-MINI-2-N4 and ESP32-S2-MINI-2-N4R2 with DigiKey and Mouser
  order links.
- Espressif's [longevity commitment](https://esp-wrover-kit.espressif.com/en/products/longevity-commitment)
  lists ESP32-S2 with a minimum 12-year commitment beginning 2020-01-01.

The modeled configuration is the N4R2 board variant. The smoke does not claim
the module's PSRAM, radio, or USB functions.
