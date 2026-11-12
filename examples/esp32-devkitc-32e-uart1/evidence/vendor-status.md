# ESP32-DevKitC-32E live availability — UART1 TX profile

Checked 2026-11-09 UTC for the exact board used by this profile.

- Espressif's current [ESP32-DevKitC V4 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32/esp32-devkitc/user_guide.html)
  lists `ESP32-DevKitC-32E` with an ESP32-WROOM-32E module and 4 MB flash. Its
  header map identifies J3 pin 12 as GPIO16, available on WROOM modules, and
  used as this smoke's UART1 TX pad.
- Espressif's current [module catalog](https://www.espressif.com/products/modules)
  lists ESP32-WROOM-32E-N4 and links it to Mouser and DigiKey. The latest
  [vendor datasheet](https://documentation.espressif.com/esp32-wroom-32e_esp32-wroom-32ue_datasheet_en.html)
  is version 2.1, dated 2026-08-05 on Espressif's
  [technical-documents page](https://www.espressif.com/en/support/documents/technical-documents/esp32-wroom).
  The current product listing and datasheet do not mark this variant NRND or
  EOL.
- Mouser's live [exact-SKU listing](https://www.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-DevKitC-32E)
  identifies manufacturer part `ESP32-DevKitC-32E`, price $10.21, minimum
  quantity 1, and showed 1,692 in stock with immediate shipment when checked.

Demand rank: **ESP32 family, rank 1** (38% of reported MCU use). This smoke
adds UART1 TX pin-level coverage to the existing exact DevKitC-32E profile; it
does not add a new board or MCU model. The UART1 default pins are avoided: the
firmware routes TX to GPIO16, which Espressif maps to the accessible J3 pin 12.
