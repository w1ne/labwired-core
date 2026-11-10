# Live vendor and buyability check

Checked live on 2026-11-10 at 21:25 UTC.

- Espressif's current [ESP32-C5-DevKitC-1 v1.2 user guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c5/esp32-c5-devkitc-1/user_guide.html)
  lists the exact `ESP32-C5-DevKitC-1-N8R8` order code, its ESP32-C5-WROOM-1
  module, 8 MB flash, and 8 MB PSRAM. It gives retail and wholesale order paths.
- Espressif's current [DevKits product page](https://www.espressif.com/products/devkits)
  lists that exact N8R8 board with a $15 sample-price reference and links to
  AliExpress, DigiKey, Mouser, and Amazon US.
- Mouser's live [Espressif listing](https://www.mouser.com/en/c/semiconductors/?m=Espressif)
  identifies Espressif Systems as the manufacturer and showed the exact
  `ESP32-C5-DevKitC-1-N8R8` at **$15**, with **882 in stock** and a buy option.
  DigiKey's exact-part page marks it **Active**, though its listing showed zero
  stock; Mouser is the current in-stock purchase path.
- Adafruit's live [BME280 STEMMA QT listing](https://www.adafruit.com/product/2652)
  showed product 2652 **in stock** at **$14.95**.

Distributor inventory is a snapshot from the check time. This evidence is for
the exact v1.2 N8R8 board and the Adafruit BME280 breakout; it does not claim
lifecycle status for every ESP32-C5 die or older DevKitC board revision.

## Verification scope

The committed local LabWired CLI smoke runs RV32IMAC firmware on the focused
ESP32-C5 model, reads BME280 ID and raw temperature over GPIO8/9-routed I2C0,
sends a UART0 marker, and observes GPIO5 output edges. The firmware is loaded
directly into HP SRAM. It does not verify ROM boot, flash/XIP, PSRAM, wireless
features beyond the modeled I2C matrix subset, or physical hardware.
