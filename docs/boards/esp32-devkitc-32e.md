# Espressif ESP32-DevKitC-32E

The exact board is Espressif's ESP32-DevKitC-32E with its 4 MB
ESP32-WROOM-32E module. The GPIO/UART profile is
[`configs/systems/esp32-devkitc-32e-gpio-smoke.yaml`](../../configs/systems/esp32-devkitc-32e-gpio-smoke.yaml).

## Modeled paths

- The base smoke in `examples/esp32-devkitc-32e/io-smoke.yaml` checks GPIO23
  transitions on J3 pin 2 and the UART0 console marker.
- Demand rank: ESP32 is rank 1 in the standing list (38% MCU use). This round
  adds PCNT unit 0 counting rising edges routed from GPIO23 through the classic
  GPIO input matrix. The smoke drives eight pulses and sets GPIO16 high only
  when the PCNT readback is eight. ESP32 is rank 1; this is low-cost integration
  of an existing PCNT model on the same board, not a new board/MCU demand pick.
- The MCPWM profile uses the existing classic ESP32 MCPWM0 model through
  ESP-IDF's legacy `mcpwm_init` and `mcpwm_set_duty` APIs. It configures a 50 Hz /
  7.5% center command on operator A and checks the compare register. The
  actuator fixture follows the duty observer; the model does not emit
  MCPWM pad-level waveforms or model servo mechanics. This smoke does not
  assert servo-angle telemetry.
- `configs/systems/esp32-devkitc-32e-bme280.yaml` adds an external BME280 model
  at I2C0 address `0x77`, with SDA=GPIO21 (J3 pin 6) and SCL=GPIO22 (J3 pin 3).
  The breakout uses 3V3/GND from J2/J3.
- The rank-1 I2C1 profile attaches the same BME280 model to I2C1 at `0x77`,
  with SDA=GPIO16 (J3 pin 12) and SCL=GPIO17 (J3 pin 11). GPIO16/17 are exposed
  on the exact WROOM-32E variant; the smoke reads the chip ID and raw
  temperature data through the second controller. This is a low-cost profile
  extension on the existing board, not a new board/MCU demand pick.
- Demand rank: ESP32 is rank 1 (38% MCU use). This coverage-depth update keeps
  the same DevKitC-32E and microSD breakout, and replaces the initialization
  check with a stock Arduino SD filesystem read. The VSPI/SPI3 card uses
  SCK=GPIO18, MISO=GPIO19, MOSI=GPIO23, CS=GPIO5; firmware mounts the fixed
  32 MiB SDHC image, reads `/README.TXT`, verifies its contents, raises GPIO17
  after mount, and raises GPIO16 after the exact file read. The model is
  read-only: it supports CMD9/CSD and CMD17 block reads, rejects writes, and
  does not accept a configurable card image.
- The earlier SPI addition connects an external 128x64 SSD1306 OLED using
  SCK=GPIO18, MISO=GPIO19, MOSI=GPIO23, CS=GPIO5, and
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
power LED also remain outside the profile. PCNT is modeled; this smoke covers
unit 0 channel 0 rising-edge counting. The glitch filter is not modeled, and
external encoders and pulse sensors are not attached. I2S0/1, UHCI0, APB_CTRL,
and DPORT analog-AHB retain fallback windows.

The rank-1 RMT smoke is in
[`examples/esp32-devkitc-32e/rmt-smoke.yaml`](../../examples/esp32-devkitc-32e/rmt-smoke.yaml)
and its complete copy-paste firmware source is
[`rmt_smoke.cpp`](../../examples/esp32-devkitc-32e/firmware/src/rmt_smoke.cpp).

The rank-1 LEDC servo smoke is in
[`examples/esp32-devkitc-32e/ledc-smoke.yaml`](../../examples/esp32-devkitc-32e/ledc-smoke.yaml)
and uses the Arduino APIs shown in
[`ledc_servo_smoke.cpp`](../../examples/esp32-devkitc-32e/firmware/src/ledc_servo_smoke.cpp).

The rank-1 MCPWM smoke is in
[`examples/esp32-devkitc-32e/mcpwm-smoke.yaml`](../../examples/esp32-devkitc-32e/mcpwm-smoke.yaml).
It configures MCPWM0 timer 0 / operator A for a 50 Hz center-servo command;
the board README has the copy-paste sketch and profile limitations.

The rank-1 SPI OLED smoke is in
[`examples/esp32-devkitc-32e/spi-oled-smoke.yaml`](../../examples/esp32-devkitc-32e/spi-oled-smoke.yaml)
with copy-paste Arduino code in the
[`DevKitC README`](../../examples/esp32-devkitc-32e/README.md). It exercises
only master TX to the panel; it does not claim SPI MISO slave coverage.

The rank-1 PCNT smoke is in
[`examples/esp32-devkitc-32e/pcnt-smoke.yaml`](../../examples/esp32-devkitc-32e/pcnt-smoke.yaml)
with copy-paste Arduino code in the
[`DevKitC README`](../../examples/esp32-devkitc-32e/README.md). It counts eight
GPIO23 rising edges and drives GPIO16 high after a matching count.

The rank-1 I2C1 BME280 smoke is in
[`examples/esp32-devkitc-32e/i2c1-bme280-smoke.yaml`](../../examples/esp32-devkitc-32e/i2c1-bme280-smoke.yaml).
Its committed local log reads BME280 ID `0x60` and temperature ADC `0x082DC0`
on GPIO16/GPIO17. The simulator smoke uses the ESP32 command-list registers;
the Arduino `Wire1` driver path is not covered by this evidence.

The ESP32 rank-1 microSD filesystem-read smoke is in
[`examples/esp32-devkitc-32e/microsd-smoke.yaml`](../../examples/esp32-devkitc-32e/microsd-smoke.yaml).
Its copy-paste Arduino example in the
[`DevKitC README`](../../examples/esp32-devkitc-32e/README.md) uses `SD.begin`
and `SD.open("/README.TXT")`. The committed local smoke checks the file bytes
and the GPIO16 pass edge. Hosted verification evidence under
`examples/esp32-devkitc-32e/evidence/microsd/hosted-verification.md` covers the
earlier initialization command profile; it does not validate this filesystem
image.

Live board, module, and sensor status is recorded in
[`examples/esp32-devkitc-32e/evidence/vendor-status.md`](../../examples/esp32-devkitc-32e/evidence/vendor-status.md).
