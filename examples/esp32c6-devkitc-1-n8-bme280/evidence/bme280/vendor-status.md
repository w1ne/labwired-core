# ESP32-C6-DevKitC-1-N8 + BME280 availability check

Checked live on 2026-11-10 at 16:56 UTC.

- Espressif's current v1.2 user guide lists the exact `ESP32-C6-DevKitC-1-N8`
  ordering part number, with an ESP32-C6-WROOM-1 and 8 MB flash. Its retail
  orders section links to Espressif's sample-order route; the guide has no
  end-of-life notice.
- DigiKey's live listing for manufacturer part `ESP32-C6-DEVKITC-1-N8` showed
  **178 in stock**, an **Add to Cart** option, and a unit price of **$9.00**.
- Adafruit's BME280 STEMMA QT breakout (product 2652) listing showed
  **In stock** at **$14.95** with an **Add to Cart** option.

Sources: [Espressif current user guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c6/esp32-c6-devkitc-1/user_guide.html),
[DigiKey exact-part listing](https://www.digikey.com/en/products/detail/espressif-systems/ESP32-C6-DEVKITC-1-N8/17728861),
and [Adafruit BME280 listing](https://www.adafruit.com/product/2652).

## Verification scope

The local repository CLI smoke connects the modeled BME280 to I2C0 on GPIO6/GPIO7,
checks its ID and a raw-temperature register sample, then captures UART0 output.
The board's physical USB-to-UART bridge is not simulated. The GPIO matrix route is
exercised; IO_MUX remains a declarative register store rather than an electrically
enforced model.

The hosted LabWired catalog describes this exact board as `design_only` with no
hosted compile path. Therefore this fixture has no hosted `labwired_verify`
verdict and is not claimed as hosted model-verified or physically tested. The
profile does not claim Wi-Fi, Bluetooth, 802.15.4, USB behavior, buttons, RGB LED
behavior, or LP-core behavior.
