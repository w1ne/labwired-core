# Vendor status and availability

Checked 2026-11-04 UTC for the Arduino Nano ESP32 board and its ESP32-S3 module.

- Arduino's [official store listing](https://store.arduino.cc/collections/boards-modules/products/nano-esp32)
  currently identifies the headered board as SKU **ABX00092**, shows **€20.40**,
  and provides **Add to cart**. It identifies the module as u-blox NORA-W106
  with an ESP32-S3. The store listing has no separate lifecycle field such as
  Active, NRND, or EOL.
- Espressif's live [ESP Product Selector](https://products.espressif.com/product-selector)
  lists **ESP32-S3R8** as **Mass Production**. This is lifecycle evidence for
  the module's SoC family, not a separate lifecycle classification for the
  complete Arduino board SKU.
- Arduino's [Nano ESP32 documentation](https://docs.arduino.cc/tutorials/nano-esp32/cheat-sheet/)
  and [official datasheet](https://docs.arduino.cc/resources/datasheets/ABX00083-datasheet.pdf)
  identify the RGB red LED on GPIO46 as active-low. The smoke exercises that
  physical GPIO directly.

The manufacturer's current store listing is orderable, and Espressif reports
the SoC in Mass Production. No obsolete or NRND component is being added. The
board profile covers the shared GPIO mapping; it does not model power, USB,
wireless, I2C, or other board functions.
