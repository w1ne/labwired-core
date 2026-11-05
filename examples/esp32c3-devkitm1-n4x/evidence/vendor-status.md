# Vendor status and availability — ESP32-C3-DevKitM-1-N4X

Checked live on 2026-11-05 for the exact board SKU.

- Espressif's current [ESP32-C3-DevKitM-1 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c3/esp32-c3-devkitm-1/user_guide.html)
  lists **ESP32-C3-DevKitM-1-N4X** as the current 4 MB variant and identifies
  the board's addressable RGB LED on GPIO8. The same table marks the older
  unsuffixed `ESP32-C3-DevKitM-1` variant **EOL**; this smoke targets N4X.
- DigiKey's live [ESP32-C3-DEVKITM-1-N4X listing](https://www.digikey.com/en/products/detail/espressif-systems/esp32-c3-devkitm-1-n4x/25587617)
  identifies Espressif Systems as the manufacturer, reports **Part Status:
  Active**, **706 in stock**, an **Add to Cart** option, and an 8-week
  manufacturer lead time at the time checked.

The narrow model covers GPIO8, IO_MUX GPIO8, and the addressable RGB LED only.
The product's radio, USB interfaces, USB-to-UART bridge, and other board
peripherals are not claimed by this profile.
