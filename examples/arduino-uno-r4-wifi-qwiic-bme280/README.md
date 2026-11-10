# Arduino UNO R4 WiFi + Qwiic BME280

This profile exercises the UNO R4 WiFi's Qwiic connector with an Adafruit
BME280 breakout. It uses the RA4M1's IIC0 controller through Arduino `Wire1`
and reads the sensor's chip ID, status, and raw temperature registers.

## Demand position

Arduino kits ranked **#3 among kits, at 37%** in the 2025 survey. This adds a
sensor path to an already supported Arduino board; it is depth work within the
kit demand signal, not a new MCU-family demand pick. The survey's ATmega328
rank does not describe the UNO R4 WiFi's Renesas RA4M1.

## Wiring

Connect the BME280 STEMMA QT cable to the board's Qwiic connector. The board
pinout maps Qwiic SDA to P401 and SCL to P400 on IIC0. Qwiic is 3.3 V only;
the BME280 breakout is compatible with Qwiic/STEMMA QT.

## Copy-paste Arduino example

This raw-register example checks the BME280 identity and reads its status and
temperature output through `Wire1`:

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t BME280_ADDRESS = 0x77;

bool readRegister(uint8_t reg, uint8_t *data, size_t length) {
  Wire1.beginTransmission(BME280_ADDRESS);
  Wire1.write(reg);
  if (Wire1.endTransmission(false) != 0) return false;
  if (Wire1.requestFrom(BME280_ADDRESS, static_cast<uint8_t>(length)) != length)
    return false;
  for (size_t i = 0; i < length; ++i) data[i] = Wire1.read();
  return true;
}

void setup() {
  Serial.begin(115200);
  Wire1.begin();  // Qwiic: SDA=P401, SCL=P400
  uint8_t id = 0, status = 0xFF, temperature[3] = {};
  if (!readRegister(0xD0, &id, 1) || id != 0x60 ||
      !readRegister(0xF3, &status, 1) || status != 0x00 ||
      !readRegister(0xFA, temperature, sizeof(temperature))) {
    Serial.println("BME280 read failed");
    return;
  }
  Serial.print("BME280 id=0x");
  Serial.print(id, HEX);
  Serial.print(" status=0x");
  Serial.print(status, HEX);
  Serial.print(" temp_raw=0x");
  for (uint8_t byte : temperature) {
    if (byte < 0x10) Serial.print('0');
    Serial.print(byte, HEX);
  }
  Serial.println();
}

void loop() {}
```

## Run the local simulator smoke

```sh
examples/arduino-uno-r4-wifi-qwiic-bme280/firmware/build.sh
scripts/onboarding_smoke.sh \
  --target-id arduino-uno-r4-wifi-qwiic-bme280 \
  --firmware-build-script examples/arduino-uno-r4-wifi-qwiic-bme280/firmware/build.sh \
  --script examples/arduino-uno-r4-wifi-qwiic-bme280/onboarding-smoke.yaml \
  --system configs/systems/arduino-uno-r4-wifi-qwiic-bme280.yaml \
  --out-dir examples/arduino-uno-r4-wifi-qwiic-bme280/evidence
```

Build logs and the structured local simulator result are committed under
[`evidence/`](evidence/). The live availability check is in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Model boundary

The focused profile models RA4M1 IIC0 and the BME280 register device. It does
not simulate Qwiic signal waveforms, the level translator, cable, voltage,
power behavior, the ESP32-S3 coprocessor, Wi-Fi/Bluetooth, USB, or the onboard
LED matrix. The reused chip profile still declares zero-answering stubs for
MSTP, flash cache, stack monitoring, and the reserved ADC unit 1 window; those
blocks are outside this sensor claim. The local simulator smoke is not a
physical-board test. The hosted verification catalog does not expose the UNO
R4 WiFi target, so this evidence is a local CLI smoke rather than a hosted
`labwired_verify` verdict.
