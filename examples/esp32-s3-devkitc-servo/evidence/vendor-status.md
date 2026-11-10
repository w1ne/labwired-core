# ESP32-S3-DevKitC-1-N16R8 live availability

Checked live on 2026-11-10 UTC.

- Espressif's [ESP32-S3-DevKitC-1 catalog page](https://www.espressif.com/en/products/devkits/esp32-s3-series/esp32-s3-devkitc-1)
  lists the exact N16R8 variant and links it to Mouser.
- Espressif's [v1.0 user guide](https://documentation.espressif.com/esp-dev-kits/en/latest/esp32s3/esp32-s3-devkitc-1/user_guide_v1.0.html)
  identifies GPIO4 as J1 pin 4. It documents GPIO48 as the RGB LED data pin on
  the v1.0 board used by this profile.
- Mouser's [exact N16R8 product listing](https://www.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-S3-DevKitC-1-N16R8)
  showed **68 in stock** and **504 on order** expected 2026-12-07. Its listing
  showed a **US$13.24** unit price and a minimum quantity of 1.

The board is listed as orderable and immediately shippable today. Check the
revision marking before using the onboard RGB LED; Espressif lists both v1.0
and v1.1, with different RGB LED GPIO pins. The external servo signal uses
GPIO4 on either revision.

Demand rank: **ESP32 family #1** (38% MCU use and 42% kits in the standing
survey). This round adds system/peripheral depth for an existing MCU model.
