# Live lifecycle and availability — ESP32-DevKitC-32E + microSD breakout

Checked 2026-11-10 UTC against current manufacturer and distributor pages.

- Espressif's [current ESP32-DevKitC V4 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32/esp32-devkitc/user_guide.html)
  lists the exact `ESP32-DevKitC-32E` with an `ESP32-WROOM-32E` module and
  4 MB flash. The exact variant is still in the current ordering table.
- Mouser's [exact ESP32-DevKitC-32E listing](https://www.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-DevKitC-32E?mgh=1&qs=GedFDFLaBXFpgD0kAZWDrQ%3D%3D)
  showed 1,692 in stock, immediate shipment, and minimum order quantity 1 in
  the current distributor listing. DigiKey marks the same MPN `Active` but
  showed zero in stock, so Mouser is the purchasable source in this check.
- The matching [ESP32-WROOM-32E datasheet](https://documentation.espressif.com/esp32-wroom-32e_esp32-wroom-32ue_datasheet_en.html)
  is the current v2.1 publication and carries no NRND/EOL status marking.
- DigiKey's [Adafruit Micro SD SPI/SDIO breakout listing, PID 4682](https://www.digikey.com/en/products/detail/adafruit-industries-llc/4682/12822319)
  marks the part `Active`, showed 2,299 in stock, and listed $3.50 at quantity
  one. The breakout is 3 V-only; use it with a separate SPI-capable microSD
  card, which is not included in the breakout.
- DigiKey's [Kingston SDCE/32GB microSD card listing](https://www.digikey.com/en/products/detail/kingston-technology/SDCE-32GB/26598480)
  marks the card `Active` and showed 175 in stock. It is a buyable 32 GB
  microSDHC example for the breakout. The simulation instead seeds a smaller
  32 MiB FAT16 image; it does not model this card's exact capacity or format.

Demand rank: **ESP32 family, rank 1** (38% measured MCU use). This is a
filesystem write/read coverage extension on the existing DevKitC-32E, not a new
board or MCU demand pick.

The local model seeds a fixed 32 MiB SDHC image with a FAT16 volume and
`/README.TXT`. It supports single-block sector writes held in simulation memory
for that run, then restores the seeded image for the next run. It does not
model flash wear or custom card images. The hosted oracle record in this
directory covers the independent hosted card-initialization profile; the
filesystem write/read evidence is the repository-local simulation output.
Stock and pricing can change after this check. This is simulator evidence, not
a physical hardware run.
