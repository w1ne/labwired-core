# Arduino Nano ESP32 + INA219 power monitor

This example wires an Adafruit INA219 high-side current monitor to the
Arduino Nano ESP32 (ABX00083, with headers). The Nano's A4/SDA and A5/SCL pins
map to GPIO11 and GPIO12. The firmware writes the INA219's configuration and
calibration registers, then reads bus voltage, current, and power over I²C.

**Demand rank:** ESP32 is #1 in the measured MCU-use survey (38%); Arduino
boards are #3 among reported project kits (37%). This is an integration path
for an already-modeled part on a high-demand board, not a new MCU model.

## Wiring

Use the Adafruit INA219 STEMMA QT breakout (product 904), with its I²C address
straps left at `0x40`:

| INA219 breakout | Arduino Nano ESP32 |
|---|---|
| VIN | 3V3 |
| GND | GND |
| SDA | A4 / GPIO11 |
| SCL | A5 / GPIO12 |
| VIN+ / VIN− | In series with the supply and load being monitored |

The local model represents the measured load as two deterministic inputs,
`bus_voltage` and `current`. The smoke changes these from 3.3 V / 0 A to 12 V /
0.25 A and checks the resulting registers. It does not model analog shunt
physics, ADC noise, conversion time, or calibration-dependent scale changes.

## Copy-paste Arduino sketch

This uses only the Arduino `Wire` library. It applies the calibration value
used by Adafruit's 0.1 Ω, 32 V / 2 A example breakout, reads the 16-bit INA219
registers, and reports milli-units:

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t kAddress = 0x40;

bool write16(uint8_t reg, uint16_t value) {
  Wire.beginTransmission(kAddress);
  Wire.write(reg);
  Wire.write(value >> 8);
  Wire.write(value & 0xFF);
  return Wire.endTransmission() == 0;
}

bool read16(uint8_t reg, uint16_t &value) {
  Wire.beginTransmission(kAddress);
  Wire.write(reg);
  if (Wire.endTransmission(false) != 0) return false;
  if (Wire.requestFrom(kAddress, static_cast<uint8_t>(2)) != 2) return false;
  value = (static_cast<uint16_t>(Wire.read()) << 8) | Wire.read();
  return true;
}

void setup() {
  Serial.begin(115200);
  Wire.begin(11, 12, 100000);  // A4/SDA, A5/SCL
  if (!write16(0x00, 0x399F) || !write16(0x05, 4096)) {
    Serial.println("INA219_I2C_ERROR");
    return;
  }

  uint16_t bus = 0, current = 0, power = 0;
  if (!read16(0x02, bus) || !read16(0x04, current) || !read16(0x03, power)) {
    Serial.println("INA219_I2C_ERROR");
    return;
  }
  Serial.printf("VBUS_MV=%lu I_MA=%ld P_MW=%lu\n",
                static_cast<unsigned long>(static_cast<uint32_t>(bus >> 3) * 4),
                static_cast<long>(static_cast<int16_t>(current) / 10),
                static_cast<unsigned long>(power * 2));
}

void loop() {}
```

## Evidence and limits

The [vendor-status receipt](evidence/vendor-status.md) records the live board
and sensor availability check. The local smoke checks the Nano's S3 I²C0 path,
INA219 configuration readback, and both input-driven measurement snapshots.
The board's radios, native USB, PSRAM, other peripherals, and physical power-on
behavior remain outside this profile. No hardware capture is claimed.

Reproduce the committed smoke from the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id arduino-nano-esp32-ina219 \
  --script examples/arduino-nano-esp32-ina219/onboarding-smoke.yaml \
  --system configs/systems/arduino-nano-esp32-ina219.yaml \
  --firmware-build-script examples/arduino-nano-esp32-ina219/ci/build-firmware.sh \
  --out-dir examples/arduino-nano-esp32-ina219/evidence
```
