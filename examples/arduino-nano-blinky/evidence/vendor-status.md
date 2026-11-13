# ATmega328P vendor lifecycle check

- Checked live: 2026-11-13, using the real Chrome profile through
  `mc-live-status.py`.
- Vendor page: [Microchip ATmega328P](https://www.microchip.com/en-us/product/ATmega328P)
- Page status: **Not Recommended for new designs** (confirmed again on
  2026-11-13).
- Arduino's live [classic Nano store listing](https://store.arduino.cc/products/arduino-nano)
  was checked 2026-11-13. It identifies SKU A000005, describes the ATmega328
  Nano 3.x, shows €19.04 (reduced from €27.20), and exposes an **Add to cart**
  control. This is board availability evidence only; it does not change the
  MCU's NRND status.
- Scope: maintenance of the existing Arduino Uno/Nano model only. This is not
  approval to onboard ATmega328P as new hardware.
