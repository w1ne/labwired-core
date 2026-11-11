# Live vendor and availability check — Adafruit QT Py RP2040

Checked 2026-11-11 UTC using Adafruit's live product page.

- Adafruit's [QT Py RP2040 product page](https://www.adafruit.com/product/4900)
  lists product 4900 for **$9.95**, shows **94 in stock**, and provides an
  **Add to Cart** control. This is current vendor sale evidence, not an
  NRND/obsolete part-status inference.
- Adafruit's [pinout guide](https://learn.adafruit.com/adafruit-qt-py-2040/pinouts)
  identifies the STEMMA QT connector as I2C1 and documents its board pin
  functions. The board's TX pad is UART1 TX.
- Adafruit's [BME280 STEMMA QT breakout page](https://www.adafruit.com/product/2652)
  lists product 2652 for **$14.95**, **In stock**, with an **Add to Cart**
  control. It is the exact solderless breakout used by this profile.

This is an RP2040 **#5 MCU demand-rank** board (12%; RP2040/RP2350 is target
group #2). The smoke reads a BME280 over I2C1 at GP22/GP23 and reports through
UART1 TX at GP20. The physical USB-C host/CDC path and onboard addressable
NeoPixel are not attached in this simulator profile; TBMAN remains a storage
stub and is not accessed.
