# Live vendor and buyability check

Checked live on 2026-11-10 at 20:27 UTC.

- Espressif's current [ESP32-C5-DevKitC-1 v1.2 user guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c5/esp32-c5-devkitc-1/user_guide.html)
  lists the exact `ESP32-C5-DevKitC-1-N8R8` order code, its ESP32-C5-WROOM-1
  module, 8 MB flash, and 8 MB PSRAM. It gives retail and wholesale order paths.
- Espressif's live [DevKits product page](https://www.espressif.com/products/devkits)
  lists that exact N8R8 board with a $15 sample price reference and links to
  AliExpress, DigiKey, Mouser, and Amazon US.
- DigiKey's live [exact-part listing](https://www.digikey.com/en/products/detail/espressif-systems/ESP32-C5-DEVKITC-1-N8R8/28718162)
  identifies Espressif Systems as the manufacturer, marks the part **Active**,
  and showed **310 in stock** at **$15.00** each for one unit.

Distributor inventory is a snapshot from the check time. This evidence is for
the exact v1.2 N8R8 board; it does not claim lifecycle status for every ESP32-C5
die or older DevKitC board revision.

## Verification scope

The committed local LabWired CLI smoke runs RV32IMAC firmware on the focused
ESP32-C5 model, sends a UART0 marker, and observes GPIO5 output edges. The
firmware is loaded directly into HP SRAM. This profile does not verify ROM boot,
flash/XIP, PSRAM, wireless features, GPIO matrix routing, or physical hardware.
