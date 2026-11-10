# Live vendor status — ESP32-DevKitC-32E

Checked 2026-11-10 against Espressif's current board page and current
distributor listings.

- Espressif's [ESP32-DevKitC product page](https://www.espressif.com/en/products/devkits/esp32-devkitc/)
  lists the exact ESP32-DevKitC-32E with an ESP32-WROOM-32E module and 4 MB
  flash. The page still presents DevKitC as a current product and links to
  purchase channels.
- The current [ESP32-DevKitC V4 guide](https://documentation.espressif.com/api/resource/path/docs/projects/esp-dev-kits/en/latest/esp32/esp32-devkitc/user_guide.html)
  lists ESP32-DevKitC-32E in its ordering table and identifies the 4 MB,
  no-PSRAM WROOM-32E variant.
- Mouser's [exact board listing](https://www.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-DevKitC-32E?mgh=1&qs=GedFDFLaBXFpgD0kAZWDrQ%3D%3D)
  identifies manufacturer part ESP32-DevKitC-32E and describes it as a
  WROOM-32E, 4 MB development kit. The page exposes a Buy/order control; live
  inventory was not visible in the page extraction.
- DigiKey's [exact board listing](https://www.digikey.com/en/products/detail/espressif-systems/ESP32-DEVKITC-32E/12091810)
  labels ESP32-DEVKITC-32E **Active**, prices one unit at $10.00, and says
  **0 In Stock — Check Lead Time**. This confirms an active orderable SKU but
  does not promise immediate dispatch. A Mouser search result captured
  2026-11-06 showed stock; that older quantity is not used as today's
  inventory claim.

The exact board and module remain listed by the manufacturer and the board is
orderable from distributors subject to lead time. Stock varies by seller; this
receipt does not claim same-day availability or physical validation.

The smoke targets only the board's on-chip MCPWM0 behavior. The servo is an
external simulation fixture and is not part of the DevKitC bill of materials.
