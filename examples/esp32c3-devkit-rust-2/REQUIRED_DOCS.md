# Source documents — ESP32-C3-DevKit-RUST-2

- Espressif's [live board guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c3/esp32-c3-devkit-rust-2/user_guide.html)
  identifies the exact board, its GPIO2 addressable RGB LED, and the onboard
  ICM-42670-P and SHTC3 sensors; it maps I2C SDA to GPIO10 and SCL to GPIO8.
- Espressif's [board schematic](https://dl.espressif.com/public/esp32-c3-devkit-rust-2-schematic.pdf)
  labels the GPIO2 RGB device WS2812B and shows the sensor wiring. The current
  revision is dated 2025-10-28.
- Sensirion's [SHTC3 datasheet v4](https://sensirion.com/media/documents/643F9C8E/63A5A436/Datasheet_SHTC3.pdf)
  documents address 0x70, command words, conversion timing, output formulas,
  and CRC-8 framing used by the sensor model and smoke.
- The [Espressif product page](https://www.espressif.com/en/dev-board/esp32-c3-devkit-rust-2-en)
  lists this board as an active product. Dated distributor availability is
  recorded in [`evidence/vendor-status.md`](evidence/vendor-status.md).
