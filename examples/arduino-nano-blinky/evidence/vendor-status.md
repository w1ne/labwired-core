# ATmega328P vendor lifecycle check

- Checked live: 2026-11-11, using the real Chrome profile through
  `mc-live-status.py`.
- Vendor page: [Microchip ATmega328P](https://www.microchip.com/en-us/product/ATmega328P)
- Page status: **Not Recommended for new designs**.
- Arduino's live [classic Nano store listing](https://store.arduino.cc/products/arduino-nano)
  identifies SKU A000005, lists the ATmega328 Nano 3.x, shows a current price,
  and exposes an **Add to cart** control (checked 2026-11-11). This is board
  availability evidence only; it does not change the MCU's NRND status.
- Scope: maintenance of the existing Arduino Uno/Nano model only. This is not
  approval to onboard ATmega328P as new hardware.
