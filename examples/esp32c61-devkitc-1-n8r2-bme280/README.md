# ESP32-C61-DevKitC-1-N8R2 + BME280

This is a demand pick: ESP32 ranks **#1** in the measured target list, at 38%
reported usage. The bare-metal smoke boots on the N8R2 DevKitC-1, configures
C61 I2C0 on GPIO6/GPIO22, reads the BME280 chip ID and raw-temperature test
vector, then prints a UART0 success marker.

## Wiring

Connect a BME280 breakout at I²C address `0x77`:

| BME280 | ESP32-C61-DevKitC-1 v2.0 |
| --- | --- |
| SDA | GPIO6 (J1 pin 5) |
| SCL | GPIO22 (J3 pin 6) |
| GND | GND |
| VIN | 3V3 |

The GPIO header locations come from Espressif's [board guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c61/esp32-c61-devkitc-1/user_guide.html).

## Copy-paste Arduino `Wire` example

This simple `Wire` sketch checks the sensor ID and reads its raw-temperature
register. Espressif currently supports C61 through Arduino as an ESP-IDF
component or rebuilt static libraries; it is not a standard board-manager
target. See the [official Arduino-ESP32 support note](https://github.com/espressif/arduino-esp32#supported-chips).
The committed simulator smoke is separate bare-metal firmware and does not
claim a physical board run.

```cpp
#include <Wire.h>

constexpr uint8_t BME280 = 0x77;

bool readRegisters(uint8_t first, uint8_t *data, size_t count) {
  Wire.beginTransmission(BME280);
  Wire.write(first);
  if (Wire.endTransmission(false) != 0) return false;
  if (Wire.requestFrom(BME280, static_cast<uint8_t>(count)) != count) return false;
  for (size_t i = 0; i < count; ++i) data[i] = Wire.read();
  return true;
}

void setup() {
  Serial.begin(115200);
  Wire.begin(6, 22);  // SDA, SCL
  uint8_t id = 0;
  uint8_t raw[3] = {};
  if (!readRegisters(0xD0, &id, 1) || id != 0x60 ||
      !readRegisters(0xFA, raw, sizeof(raw))) {
    Serial.println("BME280 read failed");
    return;
  }
  const uint32_t rawTemperature = (uint32_t(raw[0]) << 12) |
                                  (uint32_t(raw[1]) << 4) | (raw[2] >> 4);
  Serial.printf("BME280 ID=0x%02X TEMP_RAW=0x%06lX\n",
                id, static_cast<unsigned long>(rawTemperature));
}

void loop() {}
```

The BME280 simulator supplies a deterministic register test vector. This smoke
proves the I²C command engine, GPIO matrix route, sensor register read, and UART
output; it does not model environmental sampling or temperature compensation.

## Build and run

```sh
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32c61-devkitc-1-n8r2-bme280 \
  --crate firmware-esp32c61-bme280-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/esp32c61-devkitc-1-n8r2-bme280/bme280-smoke.yaml \
  --system configs/systems/esp32c61-devkitc-1-n8r2-bme280.yaml \
  --out-dir examples/esp32c61-devkitc-1-n8r2-bme280/evidence
```

See [`vendor-status.md`](vendor-status.md) for the dated availability check and
[`VALIDATION.md`](VALIDATION.md) for committed smoke evidence and model limits.
