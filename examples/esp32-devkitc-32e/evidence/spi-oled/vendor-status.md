# ESP32-DevKitC-32E and SSD1306 SPI display availability

Checked live on 2026-11-10 UTC for this board system and its display.

- Espressif's [ESP32-DevKitC V4 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32/esp32-devkitc/user_guide.html)
  identifies the current DevKitC V4 board and exposes GPIO18, GPIO19, GPIO23,
  GPIO5, and GPIO17 at J3 pins 9, 8, 2, 10, and 11 respectively. This smoke
  uses Arduino-ESP32's standard VSPI assignment: SCK=GPIO18, MISO=GPIO19,
  MOSI=GPIO23; GPIO5 is CS and GPIO17 is D/C.
- Mouser's live [ESP32-DevKitC-32E listing](https://www.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-DevKitC-32E)
  identifies the exact manufacturer SKU, with an ESP32-WROOM-32E and 4 MB
  flash. On the date checked it showed 1,692 in stock, immediate shipment,
  minimum quantity 1, and a $10.21 unit price. The listed module is not marked
  NRND or EOL.
- DigiKey's [ESP32-WROOM-32E-N4 listing](https://www.digikey.com/en/products/detail/espressif-systems/ESP32-WROOM-32E-N4/11613125)
  identifies the module's lifecycle as **Active**.
- Adafruit's live [1.3-inch 128x64 SSD1306 breakout listing](https://www.adafruit.com/product/938)
  showed the module in stock at $19.95. Adafruit specifies the SSD1306
  controller and documents SPI operation after cutting its two interface
  jumpers from the default I2C setting. The breakout has an auto-reset circuit,
  so the system omits a separate reset connection.

Demand rank: **ESP32 family, rank 1** (38% of reported MCU use). This smoke
adds the standard VSPI-to-OLED path on the existing exact DevKitC-32E board;
it is peripheral depth on the top-demand family, not a new MCU or board.
