# Live vendor status — Adafruit ESP32 Feather V2

Checked live on 2026-11-11 UTC against Adafruit's official product page. The
[product listing, PID 5400](https://www.adafruit.com/product/5400) listed the
PCB-antenna version for **$19.95**, marked it **In stock**, and showed **Add to
Cart**. The w.FL and assembled-header options were marked out of stock; this
onboarding targets the in-stock PCB-antenna variant.

Adafruit's [Feather V2 guide](https://learn.adafruit.com/adafruit-esp32-feather-v2)
identifies the red user LED as pin 13 / GPIO13 and `LED_BUILTIN`. It separately
identifies the NeoPixel on GPIO0 and the shared NeoPixel/STEMMA QT power switch
on GPIO2. This smoke covers only the red GPIO13 LED, not the NeoPixel or I2C
connector.
