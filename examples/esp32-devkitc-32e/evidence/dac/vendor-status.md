# Live vendor availability — ESP32-DevKitC-32E DAC profile

Checked live on 2026-11-09 UTC for the exact board and module used by this
profile.

- Espressif's current [ESP DevKits catalog](https://www.espressif.com/products/devkits)
  lists the exact `ESP32-DevKitC-32E` SKU and links its purchase options to
  distributors. The [current board guide](https://documentation.espressif.com/api/resource/path/docs/projects/esp-dev-kits/en/latest/esp32/esp32-devkitc/user_guide.html)
  names the same board, identifies its ESP32-WROOM-32E 4 MB module, and gives a
  retail sample ordering route. It does not mark this variant NRND or EOL.
- Espressif's current [DevKitC product page](https://www.espressif.com/en/products/devkits/esp32-devkitc/)
  lists `ESP32-DevKitC-32E` alongside other variants and maps it to
  ESP32-WROOM-32E.
- The [Mouser exact-SKU listing](https://www.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-DevKitC-32E)
  was checked on 2026-11-09 and showed 1,692 in stock and immediate shipment.
  Espressif's catalog is the lifecycle source; the distributor listing
  corroborates current buyability.
- Espressif's [ESP32 DAC guide](https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-reference/peripherals/dac.html)
  documents 8-bit DAC channels on GPIO25 and GPIO26 and the direct-output
  transfer formula. The [DevKitC V4 header table](https://documentation.espressif.com/api/resource/path/docs/projects/esp-dev-kits/en/latest/esp32/esp32-devkitc/user_guide.html)
  identifies GPIO25/DAC1 on J2 pin 9 and GPIO26/DAC2 on J2 pin 10.

Demand rank: **ESP32 family, rank 1** (38% of reported MCU usage). This adds
peripheral depth to the existing exact DevKitC-32E board profile; it is not a
new board or MCU model. The smoke uses Arduino's DAC API on the actual
ESP32 RTC_IO register path. This is deterministic model evidence, not a
physical voltage measurement.
