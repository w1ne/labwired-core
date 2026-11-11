# ESP32-DevKitC-32E + BME280 I2C smoke

This example exercises the classic ESP32 I2C0 controller on Espressif's
ESP32-DevKitC-32E. Firmware reads the attached BME280 ID (`0x60`) and raw
temperature (`0x082DC0`) over GPIO21/GPIO22, then toggles GPIO23. The smoke
gates on the GPIO edges, so an I2C read failure leaves the proof pin quiet.

The exact DevKitC-32E is currently listed by Espressif and was in stock at
Mouser when checked. See [`vendor-status.md`](vendor-status.md) for the live
availability record and source links. ESP32 is demand rank **#1** (38% of
reported MCU use); this is incremental family-depth work on an already modeled
MCU, not a new chip model.

## Copy-paste Arduino example

Connect the BME280 breakout to 3V3, GND, GPIO21 (SDA), and GPIO22 (SCL). GPIO23
is driven high only after the sensor reads succeed.

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t kBme280 = 0x77;
constexpr int kSda = 21;
constexpr int kScl = 22;
constexpr int kProofPin = 23;

bool readRegisters(uint8_t reg, uint8_t *data, size_t length) {
  Wire.beginTransmission(kBme280);
  if (Wire.write(reg) != 1 || Wire.endTransmission(false) != 0) return false;
  if (Wire.requestFrom(static_cast<uint16_t>(kBme280), length, true) != length)
    return false;
  for (size_t i = 0; i < length; ++i) {
    if (Wire.available() <= 0) return false;
    data[i] = static_cast<uint8_t>(Wire.read());
  }
  return true;
}

void setup() {
  pinMode(kProofPin, OUTPUT);
  digitalWrite(kProofPin, LOW);
  Wire.begin(kSda, kScl, 100000);

  uint8_t id = 0;
  uint8_t temp[3] = {};
  if (!readRegisters(0xD0, &id, 1) || id != 0x60 ||
      !readRegisters(0xFA, temp, sizeof(temp))) return;

  digitalWrite(kProofPin, HIGH);
}

void loop() { delay(1000); }
```

The simulation smoke and claim limits are documented in
[`VALIDATION.md`](VALIDATION.md). It does not claim Wi-Fi/Bluetooth behavior or
physical-board confirmation.
