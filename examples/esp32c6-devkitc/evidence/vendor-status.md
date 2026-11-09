# Live vendor availability — ESP32-C6-DevKitC-1-N8

Checked live on 2026-11-09:

- Espressif's live [DevKit overview](https://www.espressif.com/en/products/devkits/esp32-devkitc/overview)
  lists **ESP32-C6-DevKitC-1-N8**, a $9 sample price, and purchase links for
  AliExpress, DigiKey, Mouser, Amazon US, Adafruit, Robu India, and Akizuki.
- Espressif's [ESP32-C6-DevKitC-1 v1.2 user guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c6/esp32-c6-devkitc-1/user_guide.html)
  documents the board's GPIO9 BOOT button and GPIO8 addressable RGB LED.

The buyability check is for the exact DevKitC-1-N8 board modeled here. The
simulated board profile exercises UART0, the GPIO8 WS2812B through C6 RMT TX,
and the active-low GPIO9 BOOT button while firmware is running. It does not
model reset-time download-mode selection, radios, native USB, the external
USB-to-UART bridge, or GPIO16/GPIO17 electrical propagation.
