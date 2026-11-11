# Adafruit ESP32 Feather V2

**Demand rank:** ESP32 family, rank 1 (38% of respondents in the measured
embedded MCU survey). This extends an in-stock Feather V2 profile from GPIO to
its STEMMA QT I2C sensor connector. Live purchase and pin evidence is in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Copy-paste Arduino example: user LED

This turns on the board's red user LED (`LED_BUILTIN`, GPIO13):

```cpp
#include <Arduino.h>

void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  digitalWrite(LED_BUILTIN, HIGH);
}

void loop() { delay(100); }
```

The exact catalog board and sketch were model-verified against a GPIO13-high
oracle. The hosted profile does not capture this board's `Serial` output, so
the example asserts the LED state instead of presenting serial output as
evidence.

## Copy-paste Arduino example: STEMMA QT BME280

This reads the BME280 chip ID at register `0xD0` and turns the user LED on only
when the sensor returns `0x60`. The Adafruit BME280 breakout uses address
`0x77` with SDO high. The Feather V2 has no I2C pull-ups; the breakout has two
10 kΩ pull-ups. Firmware must enable the connector power switch on GPIO2
before starting I2C. The digital twin does not calculate resistor behavior or
gate sensor presence on GPIO2, so the smoke checks the firmware sequence and
I2C transaction, not the physical rail switch.

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t BME280_ADDRESS = 0x77;
constexpr uint8_t CHIP_ID_REGISTER = 0xD0;
constexpr uint8_t CHIP_ID = 0x60;
constexpr uint8_t STEMMA_QT_POWER = 2;

void setup() {
  pinMode(STEMMA_QT_POWER, OUTPUT);
  digitalWrite(STEMMA_QT_POWER, HIGH);
  pinMode(LED_BUILTIN, OUTPUT);
  digitalWrite(LED_BUILTIN, LOW);
  delay(10);

  Wire.begin(); // Feather V2 defaults: SDA=GPIO22, SCL=GPIO20
  Wire.beginTransmission(BME280_ADDRESS);
  Wire.write(CHIP_ID_REGISTER);
  if (Wire.endTransmission(false) != 0) return;
  if (Wire.requestFrom(BME280_ADDRESS, (uint8_t)1) != 1) return;
  if (Wire.read() == CHIP_ID) digitalWrite(LED_BUILTIN, HIGH);
}

void loop() { delay(100); }
```

The hosted verifier checks GPIO13 high after the I2C read. The committed local
smoke also checks I2C0 transfer-complete/no-NACK and ACK bits, plus the GPIO
matrix selectors that route SCL to GPIO20 and SDA to GPIO22:

Hosted oracle result and its power-model limitation are recorded in
[`evidence/bme280/hosted-verify.json`](evidence/bme280/hosted-verify.json).

```sh
scripts/onboarding_smoke.sh \
  --target-id adafruit-feather-esp32-v2-bme280 \
  --script examples/adafruit-feather-esp32-v2/bme280-smoke.yaml \
  --system configs/systems/adafruit-feather-esp32-v2-bme280.yaml \
  --out-dir examples/adafruit-feather-esp32-v2/evidence/bme280 \
  --firmware-build-script examples/adafruit-feather-esp32-v2/build-bme280-firmware.sh
```

## Local GPIO smoke

Run the existing committed GPIO fixture with:

```sh
scripts/onboarding_smoke.sh \
  --target-id adafruit-feather-esp32-v2-gpio \
  --script examples/adafruit-feather-esp32-v2/io-smoke.yaml \
  --system configs/systems/adafruit-feather-esp32-v2.yaml \
  --out-dir examples/adafruit-feather-esp32-v2/evidence/run \
  --firmware-build-script examples/adafruit-feather-esp32-v2/build-firmware.sh
```

PlatformIO builds each Arduino sketch for the exact `adafruit_feather_esp32_v2`
target. The local CLI fixtures redirect the ELF entry to a short register-level
routine to avoid depending on Arduino/ESP-IDF startup. The hosted verifier runs
the copy-paste sketches on the catalog board. Logs and structured metrics are
committed beside each example.

## Scope

The base profile claims classic ESP32 GPIO and the Feather's red GPIO13 LED.
The BME280 profile adds the existing classic ESP32 I2C0 model, a BME280 at
`0x77`, and GPIO22/GPIO20 pad routing. The local fixture performs a register
pointer read and raises GPIO13 only when the model returns chip ID `0x60`; the
hosted verifier separately runs the Arduino sketch. The digital bus model does
not calculate pull-up resistance or gate sensor presence on GPIO2. Hosted
verification assumes an ideal 3.3 V MCU rail, so power-on and brown-out reset
are not simulated. Neither profile claims Wi-Fi/Bluetooth, PSRAM, NeoPixel
behavior, or physical-board testing.
