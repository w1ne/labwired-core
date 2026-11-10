# ESP32-DevKitC-32E availability and lifecycle

Checked live on 2026-11-10 UTC for the exact board and its ESP32-WROOM-32E module.

- Espressif's [ESP32-DevKitC V4 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32/esp32-devkitc/user_guide.html)
  lists ESP32-DevKitC-32E with an ESP32-WROOM-32E module and 4 MB flash. The
  guide directs retail sample orders to Espressif's buy-a-sample page.
- Mouser's [exact ESP32-DevKitC-32E listing](https://www.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-DevKitC-32E?mgh=1&qs=GedFDFLaBXFpgD0kAZWDrQ%3D%3D)
  showed 1,692 in stock, immediate shipment, minimum quantity 1, and $10.21
  each. The exact board is available to buy today.
- DigiKey's [ESP32-WROOM-32E-N4 listing](https://www.digikey.com/en/products/detail/espressif-systems/ESP32-WROOM-32E-N4/11613125)
  showed Part Status **Active** and 6,122 in stock. This is the module used by
  the selected board. The separate older ESP32-WROOM-32 is marked NRND; this
  profile uses the WROOM-32E and does not substitute the older module.

Demand rank: **ESP32 family, rank 1** (38% of reported MCU use). This round
adds PCNT unit 0 smoke coverage to the existing exact DevKitC board; it is
peripheral-depth work, not a new board or MCU.
