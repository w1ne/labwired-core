# Live vendor status — Espressif ESP32-DevKitM-1

Checked 2026-11-10 UTC.

- Espressif's [current ESP32-DevKitM-1 product page](https://www.espressif.com/en/dev-board/esp32-devkitm-1-en)
  lists the exact board and links to DigiKey, Mouser, Amazon US, and Robu India.
  Espressif's [current user guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32/esp32-devkitm-1/user_guide.html)
  identifies it as an ESP32-MINI-1 board with 4 MB flash, USB-to-UART bridge,
  reset/download buttons, and a 3.3 V power LED.
- DigiKey's [exact ESP32-DEVKITM-1 listing](https://www.digikey.com/en/products/detail/espressif-systems/ESP32-DEVKITM-1/13532113)
  reported **Active**, **137 in stock**, **Add to Cart**, and **$8.00** when
  checked. Inventory and price can change.
- The official user guide maps U0TXD/GPIO1 and U0RXD/GPIO3 to the board
  connector and documents the USB-to-UART bridge. This smoke models the MCU
  UART0 transcript only; it does not model the bridge or certify a physical
  board.

This is a demand-rank **#1 ESP32 (38%)** board extension. It reuses the existing
ESP32 LX6/UART0 model and adds a focused profile for this distinct, current
ESP32-MINI-1 dev board.
