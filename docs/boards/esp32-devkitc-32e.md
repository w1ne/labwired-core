# Espressif ESP32-DevKitC-32E

The exact board is Espressif's ESP32-DevKitC-32E with its 4 MB
ESP32-WROOM-32E module. The GPIO/UART profile is
[`configs/systems/esp32-devkitc-32e-gpio-smoke.yaml`](../../configs/systems/esp32-devkitc-32e-gpio-smoke.yaml).

## Modeled paths

- The base smoke in `examples/esp32-devkitc-32e/io-smoke.yaml` checks GPIO23
  transitions on J3 pin 2 and the UART0 console marker.
- `configs/systems/esp32-devkitc-32e-bme280.yaml` adds an external BME280 model
  at I2C0 address `0x77`, with SDA=GPIO21 (J3 pin 6) and SCL=GPIO22 (J3 pin 3).
  The breakout uses 3V3/GND from J2/J3.
- Demand rank: ESP32 is rank 1 in the standing list (38% MCU use). This work
  adds I2C sensor depth to an existing ESP32 board.
- The smoke asserts sensor ID `0x60` and temperature raw value `0x082DC0` over
  the local UART0 capture.

Wi-Fi/Bluetooth, SPI, timers, flash boot behavior, USB-to-UART hardware,
pin-level UART waveforms, analog behavior, and the power LED remain outside the
profile. The classic ESP32 builder retains zero-answering Wi-Fi MAC/PHY,
I2S0/1, UHCI0, RMT, and PCNT windows plus APB_CTRL and DPORT analog-AHB
fallbacks; these smokes do not exercise them.

Live board, module, and sensor status is recorded in
[`examples/esp32-devkitc-32e/evidence/vendor-status.md`](../../examples/esp32-devkitc-32e/evidence/vendor-status.md).
