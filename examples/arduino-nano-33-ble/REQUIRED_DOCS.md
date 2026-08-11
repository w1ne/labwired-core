# Required Documents

- Nordic Semiconductor nRF52840 Product Specification.
- Arduino Nano 33 BLE product page and pinout:
  https://docs.arduino.cc/hardware/nano-33-ble/
- Arduino mbed core variant `ARDUINO_NANO33BLE`
  (`pins_arduino.h`, `variant.cpp`) — silkscreen → P0/P1 mapping and LED polarity.
- PlatformIO board definition `nano33ble`
  (`~/.platformio/platforms/nordicnrf52/boards/nano33ble.json`).

These documents define the memory map (via the nRF52840 chip YAML), GPIO LED
pins, stock Wire/SPI pads, and the hosted compile triple used by LabWired.
