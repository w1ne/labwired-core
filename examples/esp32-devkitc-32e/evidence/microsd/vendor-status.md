# Live lifecycle and availability — ESP32-DevKitC-32E + microSD breakout

Checked 2026-11-10 UTC against current manufacturer and distributor pages.

- Espressif's [current ESP32-DevKitC V4 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32/esp32-devkitc/user_guide.html)
  lists the exact `ESP32-DevKitC-32E` variant with an ESP32-WROOM-32E module
  and 4 MB flash. Espressif's [live DevKitC product page](https://www.espressif.com/en/products/devkits/esp32-devkitc/)
  also lists the board.
- Mouser's [exact ESP32-DevKitC-32E listing](https://www.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-DevKitC-32E?mgh=1&qs=GedFDFLaBXFpgD0kAZWDrQ%3D%3D)
  showed 1,692 in stock, immediate shipment, minimum order quantity 1, and
  $10.21 each when checked. The matching module is not an obsolete variant.
- Espressif's [current WROOM-32E datasheet](https://documentation.espressif.com/esp32-wroom-32e_esp32-wroom-32ue_datasheet_en.html)
  is v2.1 and defines v1.0-and-later datasheets as official releases for
  products in mass production. The current page carries no NRND/EOL watermark.
- Adafruit's [Micro SD SPI or SDIO Card Breakout, PID 4682](https://www.adafruit.com/product/4682)
  was listed in stock at $3.50. Its product page identifies it as a 3 V-only
  SPI/SDIO breakout; it matches the DevKitC's 3.3 V logic.

Demand rank: **ESP32 family, rank 1** (38% measured MCU use). This round adds a
common SPI storage fixture to an existing high-demand board. It is a low-cost
profile extension, not a new board/MCU demand pick.

The smoke verifies the card's SPI initialization command responses only. The
digital-twin `microsd` model has no filesystem or persistent card image.
Distributor stock is time-sensitive; these are the live observations made on
the date above, not a claim about later availability or a physical hardware run.
