# ESP32-S3-DevKitC-1 v1.1 live availability

Checked 2026-11-11 UTC against Espressif's live product documentation:

- [ESP32-S3-DevKitC-1 product page](https://www.espressif.com/en/products/devkits/esp32-s3-series/esp32-s3-devkitc-1)
  identifies the board as an entry-level ESP32-S3-WROOM-1 development board and
  exposes a **Buy** path.
- [ESP32-S3-DevKitC-1 v1.1 user guide](https://documentation.espressif.com/esp-dev-kits/en/latest/esp32s3/esp32-s3-devkitc-1/user_guide_v1.1.html)
  lists `ESP32-S3-DevKitC-1-N16R8` as a 16 MB flash / 8 MB PSRAM variant. The
  same table marks the older N8 and N8R2 variants **EOL**; N16R8 is listed among
  available variants without an EOL mark. The guide also says retail orders are
  available through Espressif's sample-order path.
- The guide identifies the onboard USB-to-UART bridge and maps J3 TX/RX to
  GPIO43/U0TXD and GPIO44/U0RXD. For v1.1 it maps the addressable RGB LED to
  GPIO38; the v1.0 guide maps it to GPIO48.

The model target is specifically the live N16R8 v1.1 variant. The current
UART profile claims boot plus UART0. The additional RGB profile exercises the
v1.1 WS2812 LED on GPIO38; it does not claim PSRAM behavior.

Demand position: **ESP32 family rank 1**, used by 38% of respondents in the
measured embedded MCU survey; ESP32 kits were used in 42% of respondents' main
projects.
