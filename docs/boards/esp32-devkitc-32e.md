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
- Demand rank: ESP32 is rank 1 in the standing list (38% MCU use). This round
  extends classic RMT TX on an existing ESP32 board to two concurrent channels.
- The smoke asserts sensor ID `0x60` and temperature raw value `0x082DC0` over
  the local UART0 capture.

The RMT smoke sends red and green WS2812 frames from channels 0 and 1 through
the GPIO matrix on GPIO23 and GPIO18. It waits for both timed TX_END events and
attaches two generic WS2812-compatible pixels as external devices; they are
test fixtures, not built-in board LEDs. The classic RMT model implements the
register window, RMTMEM, independent channels 0–7 transmit timing, GPIO-matrix
outputs and per-channel TX_END. Receive mode remains outside the modeled scope.
Wi-Fi/Bluetooth, SPI, timers, flash boot
behavior, USB-to-UART hardware, pin-level UART waveforms, analog behavior, and
the power LED also remain outside the profile. I2S0/1, UHCI0, PCNT, APB_CTRL
and DPORT analog-AHB retain fallback windows.

The rank-1 RMT smoke is in
[`examples/esp32-devkitc-32e/rmt-smoke.yaml`](../../examples/esp32-devkitc-32e/rmt-smoke.yaml)
and its complete copy-paste firmware source is
[`rmt_smoke.cpp`](../../examples/esp32-devkitc-32e/firmware/src/rmt_smoke.cpp).

Live board, module, and sensor status is recorded in
[`examples/esp32-devkitc-32e/evidence/vendor-status.md`](../../examples/esp32-devkitc-32e/evidence/vendor-status.md).
