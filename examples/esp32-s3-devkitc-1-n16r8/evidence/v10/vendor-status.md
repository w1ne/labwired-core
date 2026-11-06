# ESP32-S3-DevKitC-1 v1.0 availability

Checked live on 2026-11-06 UTC.

- Espressif's current [ESP32-S3 DevKit catalog](https://www.espressif.com/en/products/devkits/esp32-devkitc/overview)
  lists the ESP32-S3-DevKitC-1 N16R8 and links it to Mouser. The listing does
  not identify a PCB revision.
- Espressif's current [hardware FAQ](https://docs.espressif.com/projects/esp-faq/en/latest/hardware-related/development-board.html)
  says the v1.0 RGB LED is connected to GPIO48 and the v1.1 LED is connected to
  GPIO38.
- Espressif's current [DevKitC-1 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32s3/esp32-s3-devkitc-1/user_guide_v1.1.html)
  states that both the initial revision and v1.1 are available on the market.
  Its N16R8 ordering variant is listed in the official catalog above.

The vendor lists the N16R8 board as buyable without a revision selector; its
guide explicitly confirms that v1.0 boards remain on the market. The catalog
does not specify which N16R8 stock is v1.0 versus v1.1, so buyers should
inspect the board's revision marking and use the matching GPIO profile.

Demand rank: **ESP32 family #1** in the standing evidence (38% reported MCU
use; 42% ESP32 kits). This is board-revision coverage on an existing model,
not a new MCU core.
