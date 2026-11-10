# Arduino Nano ESP32 + BME280

**Demand rank:** ESP32 is #1 (38%); Arduino kits are used by 37% of surveyed
projects. This adds board-level coverage for the Arduino Nano ESP32
(ABX00092), using its ESP32-S3 and a real I2C BME280 model.

## Modelled smoke

The firmware reads the BME280 chip ID (`0xD0` → `0x60`) and raw temperature
registers (`0xFA`–`0xFC`) over I2C0. The accepted output is:

```text
ARDUINO_NANO_ESP32_BME280_OK ID=0x60 TEMP_RAW=0x082DC0 I2C0=A4(GPIO11)/A5(GPIO12)
```

The local smoke build routes `Serial` to UART0 so the CLI can capture the
acceptance marker. A stock Nano ESP32 Arduino build can keep USB CDC enabled;
the sketch writes once through `Serial`, so the active console receives one
clean copy. Availability checks are recorded in [vendor-status.md](vendor-status.md).

## Run

From the repository root:

```sh
examples/arduino-nano-esp32-bme280/firmware/build-firmware.sh
cargo run -q -p labwired-cli -- test \
  --script examples/arduino-nano-esp32-bme280/bme280-smoke.yaml \
  --output-dir out/arduino-nano-esp32-bme280 \
  --no-uart-stdout
```

The checked-in `firmware/src/main.cpp` is the copy-paste Arduino sketch.

## Wiring

This example uses the board's default I2C pins, A4 and A5. The local simulator
build uses the generic ESP32-S3 Arduino variant so it can skip the Nano
variant's startup recovery/RGB sequence; the sketch selects the matching raw
GPIOs for that build and A4/A5 when compiled with the Nano pin-remap variant.

| BME280 STEMMA QT breakout | Arduino Nano ESP32 |
| --- | --- |
| VIN | 3V3 |
| GND | GND |
| SDA | A4 / GPIO11 |
| SCL | A5 / GPIO12 |

The BME280 is configured at I2C address `0x77`.

## Coverage limits

- The simulation exercises the ESP32-S3 I2C controller and BME280 register
  model. The manifest records the A4/A5 route, but the simulator does not
  prove physical pull-ups, voltage levels, board wiring, or silicon parity.
- Wi-Fi, Bluetooth, the onboard RGB LEDs, native USB electrical behavior,
  analog inputs, and the other Nano headers are outside this smoke. The Nano
  variant's recovery/RGB startup sequence, OPI PSRAM, USB CDC/DFU, and pin
  remapping are not included in the local firmware profile.
- The Nano ESP32 is an already-modelled ESP32-S3 system; this is new board
  coverage, not a new MCU-family model.

See [VALIDATION.md](VALIDATION.md) for the run evidence and
[EXTERNAL_COMPONENTS.md](EXTERNAL_COMPONENTS.md) for the sensor-model boundary.
