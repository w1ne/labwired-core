# Seeed Studio XIAO ESP32-C3 vendor status

Checked live on **2026-11-05**.

- Seeed's [XIAO ESP32-C3 product page](https://www.seeedstudio.com/Seeed-XIAO-ESP32C3-p-5431.html)
  listed SKU **113991054** at **$4.99**, marked **In stock**, with an **Add to
  Cart** control.
- Seeed's [Getting Started guide](https://wiki.seeedstudio.com/XIAO_ESP32C3_Getting_Started/)
  maps board pin **D10** to **GPIO10** and demonstrates blinking an external
  LED from D10 with a series resistor. It also states that the board has no
  `LED_BUILTIN` user LED.
- Espressif's [ESP32-C3 module page](https://www.espressif.com/en/products/modules?id=ESP32-C3)
  lists ESP32-C3-MINI-1 order links at DigiKey and Mouser. The model in this
  change covers the C3 GPIO block used by D10; it does not claim the module's
  radio or other blocks.

The purchase-status evidence is the current Seeed board listing, rather than a
historical or archived page.
