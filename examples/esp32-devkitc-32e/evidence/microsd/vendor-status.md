# Live lifecycle and availability — ESP32-DevKitC-32E + microSD breakout

Checked 2026-11-10 UTC against current manufacturer and distributor pages.

- Espressif's [current ESP32-DevKitC V4 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32/esp32-devkitc/user_guide.html)
  lists the exact `ESP32-DevKitC-32E` with an `ESP32-WROOM-32E` module and
  4 MB flash. The exact variant is still in the current ordering table.
- Mouser's [exact ESP32-DevKitC-32E listing](https://www.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-DevKitC-32E?mgh=1&qs=GedFDFLaBXFpgD0kAZWDrQ%3D%3D)
  showed 1,692 in stock, immediate shipment, minimum order quantity 1, and
  $10.21 each. DigiKey marks the same MPN `Active` but showed zero in stock
  and an estimated future arrival, so Mouser is the current purchasable source.
- The matching [ESP32-WROOM-32E datasheet](https://documentation.espressif.com/esp32-wroom-32e_esp32-wroom-32ue_datasheet_en.html)
  is the current v2.1 publication and carries no NRND/EOL status marking.
- DigiKey's [Adafruit Micro SD SPI/SDIO breakout listing, PID 4682](https://www.digikey.com/en/products/detail/adafruit-industries-llc/4682/12822319)
  marks the part `Active`, showed 2,299 in stock, and listed $3.50 at quantity
  one. The breakout is 3 V-only; use it with a separate SPI-capable microSD
  card, which is not included in the breakout.

Demand rank: **ESP32 family, rank 1** (38% measured MCU use). This is a
filesystem-read coverage extension on the existing DevKitC-32E, not a new
board or MCU demand pick.

The local model provides a fixed 32 MiB read-only SDHC image with a FAT16
volume and `/README.TXT`. It rejects writes and does not accept custom card
images. The hosted oracle record in this directory covers the earlier card
initialization profile; the current filesystem-read evidence is the committed
repository-local simulation output. Stock and pricing can change after this
check. This is simulator evidence, not a physical hardware run.
