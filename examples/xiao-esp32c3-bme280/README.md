# Seeed XIAO ESP32-C3 + BME280

This board profile targets the ESP32 family, demand rank **#1** (38% in the
JetBrains State of Embedded survey). It adds a board profile for the existing
ESP32-C3 model; it does not add a new chip model.

The smoke boots Arduino firmware, reads BME280 identity `0x60` and the modeled
raw temperature `0x082DC0` over I²C0 on D4/D5 (GPIO6/7), then toggles D10
(GPIO10). The smoke observes four D10 transitions; firmware reaches that loop
only after both sensor values match. USB Serial/JTAG, wireless, battery
charging, and the power-only charge LED are outside this tested profile.

## Wire the example

Connect a BME280 breakout to 3V3, GND, D4/SDA, and D5/SCL. The I²C address in
this example is `0x77`. To observe the proof output physically, connect an LED
and a series current-limiting resistor from D10 to GND. D10 is a header output,
not an onboard indicator.

## Copy-paste Arduino sketch

This sketch reads the BME280 ID and temperature, then blinks an external LED
on D10 only if both match. Connect the LED through a series resistor.

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t kBme280Address = 0x77;
constexpr int kSdaPin = 6;    // D4
constexpr int kSclPin = 7;    // D5
constexpr int kProofPin = 10; // D10, external LED + resistor
bool sensorReadPassed = false;

bool readRegisters(uint8_t reg, uint8_t *data, size_t length) {
  Wire.beginTransmission(kBme280Address);
  if (Wire.write(reg) != 1 || Wire.endTransmission(false) != 0) return false;
  if (Wire.requestFrom(static_cast<uint16_t>(kBme280Address), length, true) !=
      length) return false;
  for (size_t i = 0; i < length; ++i) {
    if (Wire.available() <= 0) return false;
    data[i] = static_cast<uint8_t>(Wire.read());
  }
  return true;
}

void setup() {
  pinMode(kProofPin, OUTPUT);
  digitalWrite(kProofPin, LOW);
  Wire.begin(kSdaPin, kSclPin, 100000);

  uint8_t id = 0;
  if (!readRegisters(0xD0, &id, 1) || id != 0x60) return;

  uint8_t temperature[3] = {};
  if (!readRegisters(0xFA, temperature, sizeof(temperature))) return;
  const uint32_t raw = (uint32_t(temperature[0]) << 12) |
                       (uint32_t(temperature[1]) << 4) |
                       (uint32_t(temperature[2]) >> 4);
  sensorReadPassed = raw == 0x082DC0;
}

void loop() {
  if (!sensorReadPassed) return;
  digitalWrite(kProofPin, HIGH);
  delay(2);
  digitalWrite(kProofPin, LOW);
  delay(2);
}
```

The same sketch is committed at
[`firmware/src/main.cpp`](firmware/src/main.cpp).

Run the committed LabWired smoke:

```sh
examples/xiao-esp32c3-bme280/firmware/build-firmware.sh
LABWIRED_ESP32C3_FLASH="$PWD/examples/xiao-esp32c3-bme280/firmware/.pio/build/xiao_c3/merged.bin" \
  cargo run -q -p labwired-cli -- test \
    --script examples/xiao-esp32c3-bme280/bme280-smoke.yaml \
    --output-dir out/xiao-esp32c3-bme280 \
    --rom-boot --no-uart-stdout
```

See [`vendor-status.md`](vendor-status.md) for the live availability check.
The committed [`evidence/`](evidence/) directory contains the local smoke logs
and structured result. This is model evidence, not a physical board test.
