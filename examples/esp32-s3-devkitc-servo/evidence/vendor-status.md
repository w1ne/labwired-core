# ESP32-S3-DevKitC-1-N16R8 live availability

Checked live on 2026-11-06 UTC.

- Espressif's [ESP32-S3-DevKitC-1 catalog page](https://www.espressif.com/en/products/devkits/esp32-s3-series/esp32-s3-devkitc-1)
  lists the N16R8 board and links it to distributors.
- Espressif's [current v1.1 user guide](https://documentation.espressif.com/esp-dev-kits/en/latest/esp32s3/esp32-s3-devkitc-1/user_guide_v1.1.html)
  says both the initial and v1.1 board revisions remain available on the
  market. It assigns GPIO4 to J1 pin 4.
- Mouser's [exact N16R8 listing](https://www.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-S3-DevKitC-1-N16R8)
  showed **163 in stock**, **504 on order** for 2026-12-07, a unit price of
  **US$13.24**, and minimum quantity 1. The stock was listed as immediately
  shippable.

The local system uses the v1.0 GPIO48 RGB LED wiring. Espressif says initial
and v1.1 units are both on the market, but the N16R8 product listing does not
identify PCB revision. Check the board's revision marking before using its
onboard RGB LED; the external servo uses GPIO4 on either revision.

Demand rank: **ESP32 family #1** (38% MCU use and 42% kits in the standing
survey). This is system/peripheral depth for an existing MCU model, not a new
MCU or vendor.
