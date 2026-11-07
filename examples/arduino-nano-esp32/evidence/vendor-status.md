# Vendor status and availability

Checked 2026-11-05 UTC for the Arduino Nano ESP32 board and its ESP32-S3 module.

- Arduino's [official store listing](https://store.arduino.cc/products/nano-esp32)
  identifies the headered board as SKU **ABX00092**, shows **€20.40**, and
  provides **Add to cart**. It identifies the module as u-blox NORA-W106
  with an ESP32-S3. The store listing has no separate lifecycle field such as
  Active, NRND, or EOL.
- Espressif's live [ESP Product Selector](https://products.espressif.com/product-selector)
  lists **ESP32-S3R8** as **Mass Production**. This is lifecycle evidence for
  the module's SoC family, not a separate lifecycle classification for the
  complete Arduino board SKU.
- Arduino's [Nano ESP32 documentation](https://docs.arduino.cc/tutorials/nano-esp32/cheat-sheet/)
  identifies the RGB LED contacts as active-low. The [official Arduino core pin
  definitions](https://github.com/espressif/arduino-esp32/blob/master/variants/arduino_nano_nora/pins_arduino.h)
  map them to GPIO46, GPIO0, and GPIO45. Arduino's [help article](https://support.arduino.cc/hc/en-us/articles/9589073738012-About-Nano-ESP32-boards-with-inverted-green-and-blue-pins)
  documents a limited early batch with the green and blue contacts swapped; the
  smoke asserts the three physical pads without assigning a guaranteed color
  to GPIO0 or GPIO45.

The manufacturer's current store listing is orderable, and Espressif reports
the SoC in Mass Production. No obsolete or NRND component is being added. The
board profile covers the RGB GPIO contacts; it does not model power, USB,
wireless, I2C, or other board functions.

Demand rank for this round: ESP32 family **#1** (38% reported MCU use); Arduino
boards were **37%** of the reported project kits. This extends a popular board's
onboard RGB GPIO coverage and does not add a new chip.
