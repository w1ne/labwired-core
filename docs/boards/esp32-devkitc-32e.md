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
  adds VSPI/SPI3 TX to an external 128x64 SSD1306 SPI OLED on the same board.
- The SPI system uses SCK=GPIO18, MISO=GPIO19, MOSI=GPIO23, CS=GPIO5, and
  D/C=GPIO17. Its smoke sends the controller initialization sequence and
  paints the full top 8-pixel page, asserted from the OLED framebuffer.
- The smoke asserts sensor ID `0x60` and temperature raw value `0x082DC0` over
  the local UART0 capture.
- `configs/systems/esp32-devkitc-32e-ledc-servo.yaml` adds the classic LEDC
  model and an external SG90-class servo twin on GPIO23 / J3 pin 2. The smoke
  commands 49.92 Hz PWM with a 1.449 ms pulse, then checks the LEDC duty shadow
  and pad edges. The servo is a simulation fixture; physical power and motion
  are not modeled.

The RMT smoke sends red and green WS2812 frames from channels 0 and 1 through
the GPIO matrix on GPIO23 and GPIO18. It waits for both timed TX_END events and
attaches two generic WS2812-compatible pixels as external devices; they are
test fixtures, not built-in board LEDs. The classic RMT model implements the
register window, RMTMEM, independent channels 0–7 transmit timing, GPIO-matrix
outputs and per-channel TX_END. This smoke exercises channels 0 and 1; channels
2–7 use the same path but are not exercised here. Receive mode remains outside
the modeled scope.
Wi-Fi/Bluetooth, general-purpose timer APIs, flash boot behavior,
USB-to-UART hardware, pin-level UART waveforms, full analog behavior, and the
power LED also remain outside the profile. I2S0/1, UHCI0, PCNT, APB_CTRL and
DPORT analog-AHB retain fallback windows.

The rank-1 RMT smoke is in
[`examples/esp32-devkitc-32e/rmt-smoke.yaml`](../../examples/esp32-devkitc-32e/rmt-smoke.yaml)
and its complete copy-paste firmware source is
[`rmt_smoke.cpp`](../../examples/esp32-devkitc-32e/firmware/src/rmt_smoke.cpp).

The rank-1 LEDC servo smoke is in
[`examples/esp32-devkitc-32e/ledc-smoke.yaml`](../../examples/esp32-devkitc-32e/ledc-smoke.yaml)
and uses the Arduino APIs shown in
[`ledc_servo_smoke.cpp`](../../examples/esp32-devkitc-32e/firmware/src/ledc_servo_smoke.cpp).

The rank-1 SPI OLED smoke is in
[`examples/esp32-devkitc-32e/spi-oled-smoke.yaml`](../../examples/esp32-devkitc-32e/spi-oled-smoke.yaml)
with copy-paste Arduino code in the
[`DevKitC README`](../../examples/esp32-devkitc-32e/README.md). It exercises
only master TX to the panel; it does not claim SPI MISO slave coverage.

Live board, module, and sensor status is recorded in
[`examples/esp32-devkitc-32e/evidence/vendor-status.md`](../../examples/esp32-devkitc-32e/evidence/vendor-status.md).
