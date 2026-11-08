# Live vendor availability — ESP32-C6-DevKitC-1-N8

Checked live on 2026-11-05:

- Espressif's [ESP32-C6-DevKitC-1 v1.2 user guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c6/esp32-c6-devkitc-1/user_guide.html)
  lists **ESP32-C6-DevKitC-1-N8** as the available variant, describes the
  board's retail and wholesale order paths, and identifies the built-in
  addressable RGB LED on GPIO8.
- DigiKey's [ESP32-C6-DEVKITC-1-N8 listing](https://www.digikey.ca/en/products/detail/espressif-systems/ESP32-C6-DEVKITC-1-N8/17728861)
  showed manufacturer **Espressif Systems**, **Part Status: Active**, and
  **325 in stock** when checked. Listed unit price was **CAD 14.25**.

The buyability check is for the exact DevKitC-1-N8 board modeled here. The
simulated board profile exercises UART0 and the GPIO8 WS2812B via bit-banged
GPIO output. RMT, radios, BOOT button, native USB, the external USB-to-UART
bridge, and GPIO16/GPIO17 pad propagation remain outside this smoke.
