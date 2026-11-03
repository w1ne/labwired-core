# Vendor status and availability

Checked 2026-11-03 UTC for Arduino Nano ESP32, SKU ABX00083.

- Arduino's [official store listing](https://store.arduino.cc/collections/most-popular/products/nano-esp32-with-headers)
  identifies SKU ABX00083, lists the board at €21.60, and showed **Add to cart**.
  It identifies the module as u-blox NORA-W106 (ESP32-S3). The listing does not
  publish a lifecycle field such as Active, NRND, or EOL.
- Espressif's live [product selector](https://products.espressif.com/product-selector)
  lists ESP32-S3R8 as **Mass Production**. This is lifecycle evidence for the
  board's SoC family, not a separate lifecycle statement for Arduino's complete
  board SKU.
- Arduino's [Nano ESP32 documentation](https://docs.arduino.cc/tutorials/nano-esp32/cheat-sheet/)
  and [board pinout](https://docs.arduino.cc/resources/pinouts/ABX00083-full-pinout.pdf)
  map the red RGB LED to GPIO46 and show it as active-low. The smoke uses that
  physical GPIO directly.

The board was orderable from its manufacturer on the check date, and the SoC
family is listed in Mass Production. No obsolete or NRND part is being added.
