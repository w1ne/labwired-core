# Live vendor status — ESP32-DevKitC-32E

Checked live on 2026-11-09 UTC for the exact board used by the LEDC servo
smoke.

- Espressif's [ESP32 DevKits catalog](https://www.espressif.com/products/devkits)
  lists the ESP32-DevKitC and exact `ESP32-DevKitC-32E` SKU, with purchase links
  to multiple distributors.
- Mouser's [exact ESP32-DevKitC-32E listing](https://www.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-DevKitC-32E)
  identifies Espressif as the manufacturer, specifies the ESP32-WROOM-32E 4 MB
  configuration, and showed **1,692 in stock, can ship immediately** when
  checked.
- Espressif's [current ESP32-DevKitC V4 guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32/esp32-devkitc/user_guide.html)
  identifies the `ESP32-DevKitC-32E` as the ESP32-WROOM-32E 4 MB variant and
  maps J3 pin 2 to GPIO23, the PWM output used by this profile.

The DevKitC-32E remains currently listed and purchasable. This is a peripheral
coverage round on an existing rank-1 ESP32 board, not a new part onboarding.
