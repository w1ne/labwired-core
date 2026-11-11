# Live vendor and availability check — ESP32-DevKitM-1

Checked 2026-11-11 UTC.

- Espressif's [current product page](https://www.espressif.com/en/dev-board/esp32-devkitm-1-en) lists ESP32-DevKitM-1 and provides DigiKey, Mouser, and Amazon US retail links.
- Espressif's [current user guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32/esp32-devkitm-1/user_guide.html) identifies the exact board as ESP32-MINI-1 with 4 MB flash and documents its USB-to-UART bridge and exposed GPIO header.
- DigiKey's [ESP32-DEVKITM-1 listing](https://www.digikey.com/en/products/detail/espressif-systems/ESP32-DEVKITM-1/13532113) reported **Active**, **136 in stock**, **Add to Cart**, and **$8.00 USD** at check time. Availability and price can change.

This board extension is a demand-rank **#1 ESP32 (38%)** pick. The firmware smoke exercises UART0 and an external LED on GPIO23. The simulator captures UART at the MCU and does not model the physical USB-to-UART bridge, wireless radios, regulator, or power LED.
