# Arduino Zero vendor status

Checked live on 2026-11-08.

- **ATSAMD21G18 lifecycle:** ran `mc-live-status.py` (real Chrome over the
  DevTools port) against <https://www.microchip.com/en-us/product/ATSAMD21G18>.
  The rendered page reads **"Status: In Production"**. The orderable-code URL
  (`.../product/ATSAMD21G18A`) redirects to Microchip's home page, so the
  family page is the one that carries the status. No Wayback snapshot was used.
- **Board availability:** the Arduino Official Store's product JSON
  (<https://store.arduino.cc/products/arduino-zero.js>) lists "Arduino Zero",
  SKU **ABX00003**, `available: true`, price 4890 (EUR 48.90).

The lifecycle check is for the MCU on the board; the store listing is for the
board itself.
