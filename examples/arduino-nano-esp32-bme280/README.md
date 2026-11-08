# Arduino Nano ESP32 + BME280 I²C smoke

This profile adds a real BME280 I²C path to the popular Arduino Nano ESP32
(headered SKU ABX00092, ESP32-S3). Arduino documents the Nano's default I²C
header pins as A4/SDA = GPIO11 and A5/SCL = GPIO12. The board and sensor system
descriptor is
[`configs/systems/arduino-nano-esp32-bme280.yaml`](../../configs/systems/arduino-nano-esp32-bme280.yaml).

## Copy-paste Arduino example

The complete sketch is [`src/main.ino`](src/main.ino). It explicitly selects
the Nano's documented GPIO pins, reads the BME280 chip ID and temperature ADC,
and prints both values:

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t kAddress = 0x77;

bool readRegisters(uint8_t first, uint8_t *bytes, size_t count) {
  Wire.beginTransmission(kAddress);
  Wire.write(first);
  if (Wire.endTransmission(false) != 0) return false;
  if (Wire.requestFrom(kAddress, static_cast<uint8_t>(count)) != count) {
    return false;
  }
  for (size_t i = 0; i < count; ++i) bytes[i] = Wire.read();
  return true;
}

void printHexByte(uint8_t value) {
  constexpr char kHex[] = "0123456789ABCDEF";
  Serial.print(kHex[value >> 4]);
  Serial.print(kHex[value & 0x0F]);
}

void printHex20(uint32_t value) {
  printHexByte(static_cast<uint8_t>(value >> 16));
  printHexByte(static_cast<uint8_t>(value >> 8));
  printHexByte(static_cast<uint8_t>(value));
}

void setup() {
  Serial.begin(115200);
  Wire.begin(11, 12, 100000);  // A4/SDA=GPIO11, A5/SCL=GPIO12

  uint8_t id = 0;
  if (!readRegisters(0xD0, &id, 1)) {
    Serial.println("BME280_I2C_ERROR");
    return;
  }
  Serial.print("BME280_ID=0x");
  printHexByte(id);
  Serial.println();
  if (id != 0x60) return;

  uint8_t temperature[3] = {};
  if (!readRegisters(0xFA, temperature, sizeof(temperature))) {
    Serial.println("BME280_I2C_ERROR");
    return;
  }
  const uint32_t raw = (static_cast<uint32_t>(temperature[0]) << 12) |
                       (static_cast<uint32_t>(temperature[1]) << 4) |
                       (temperature[2] >> 4);
  Serial.print("BME280_TEMP_RAW=0x");
  printHex20(raw);
  Serial.println();
}

void loop() {}
```

Connect the breakout's VIN to 3V3, GND to GND, SDA to A4/GPIO11, and SCL to
A5/GPIO12. Set the BME280 address-select pin SDO high for address `0x77`.

## Model scope

The descriptor includes the ESP32-S3 GPIO matrix, I²C0, UART0, and the BME280
model at `0x77`. The local firmware performs I²C write-pointer/repeated-start
reads of ID register `0xD0` and temperature registers `0xFA`–`0xFC`; the smoke
checks `BME280_ID=0x60` and `BME280_TEMP_RAW=0x082DC0`.

The ESP32-S3 CPU and BME280 behavior are model-only; this is not a physical
board capture. Wi-Fi/Bluetooth, PSRAM, native USB, ADC, timers, SPI, power-up
behavior, and other peripherals are outside the profile. The hosted catalog's
default I²C route remains GPIO8/GPIO9; the sketch explicitly selects the
manufacturer-documented GPIO11/GPIO12 path.

The [vendor-status receipt](evidence/vendor-status.md) records current board,
SoC, and sensor availability. The [part-model record](evidence/part-model.json)
records the modeled identities and pins.

## Reproduce the local smoke

From the repository root:

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id arduino-nano-esp32-bme280 \
  --script examples/arduino-nano-esp32-bme280/onboarding-smoke.yaml \
  --system configs/systems/arduino-nano-esp32-bme280.yaml \
  --firmware-build-script examples/arduino-nano-esp32-bme280/ci/build-firmware.sh \
  --out-dir examples/arduino-nano-esp32-bme280/evidence
```

The build script compiles the Arduino sketch and the small Xtensa fast-boot
firmware used by the repository simulator. The committed smoke logs and
structured result are under `evidence/logs/` and `evidence/simulation-output/`.
