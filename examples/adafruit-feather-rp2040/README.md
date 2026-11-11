# Adafruit Feather RP2040 + BME280

Demand rank: **RP2040, rank 5** in the measured MCU list (12%). This adds a
Feather board profile and extends the RP2040 chip descriptor to expose its
second I2C controller. The Feather's STEMMA QT connector is connected to I2C1;
vendor and lifecycle evidence is in [`vendor-status.md`](vendor-status.md).

## Hardware and wiring

Use an Adafruit Feather RP2040 (PID 4884) and an Adafruit BME280 STEMMA QT
breakout (PID 2652). Connect the breakout to the Feather's STEMMA QT connector
with a QT cable. The board's default BME280 address is `0x77`.

The profile models the board's RP2040, its 8 MiB external flash capacity, the
I2C1 controller, and the BME280 chip-ID transaction. The STEMMA QT route is
I2C1 SDA=GP2 / SCL=GP3. It does not claim to model the onboard NeoPixel,
buttons, USB CDC, charger, or electrical QSPI flash behavior.

## Copy-paste Arduino example

Select **Adafruit Feather RP2040** in PlatformIO or the Arduino-Pico core,
connect the BME280 breakout over STEMMA QT, and upload:

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t BME280_ADDRESS = 0x77;
constexpr uint8_t CHIP_ID_REGISTER = 0xD0;

void setup() {
  Serial.begin(115200);
  Wire.begin(); // Feather STEMMA QT is I2C1 on GP2/GP3.
  Wire.setClock(100000);

  Wire.beginTransmission(BME280_ADDRESS);
  Wire.write(CHIP_ID_REGISTER);
  if (Wire.endTransmission(false) != 0 ||
      Wire.requestFrom(BME280_ADDRESS, static_cast<uint8_t>(1)) != 1) {
    Serial.println("BME280 not found at 0x77");
    return;
  }

  if (Wire.read() == 0x60) {
    Serial.println("FEATHER_RP2040_BME280_OK I2C1=GP2/GP3 ID=0x60");
  } else {
    Serial.println("Unexpected BME280 chip ID");
  }
}

void loop() { delay(100); }
```

## Build and simulate

```bash
examples/adafruit-feather-rp2040/build-firmware.sh
scripts/onboarding_smoke.sh \
  --target-id adafruit-feather-rp2040-bme280 \
  --script examples/adafruit-feather-rp2040/io-smoke.yaml \
  --system configs/systems/adafruit-feather-rp2040-bme280.yaml \
  --out-dir examples/adafruit-feather-rp2040/evidence/run \
  --firmware-build-script examples/adafruit-feather-rp2040/build-firmware.sh
```

The PlatformIO build compiles the complete copy-paste Arduino `Wire` sketch
for the exact Feather RP2040 board target. The simulator fixture is a separate
freestanding C image with a real Cortex-M vector table. It initializes I2C1,
reads BME280 register `0xD0`, and raises header pin D13 only when the returned
ID is `0x60`. The simulator currently cannot complete Arduino startup: its
periodic RP2040 timer-alarm reprogramming exceeds the event scheduler's
live-event ceiling. The smoke verifies the Feather's modeled I2C1, sensor, and
GPIO path, and the build log verifies that the customer-facing Arduino sketch
compiles. It does not claim that the Arduino runtime boots in simulation. The
sensor's environmental conversion values, analog pull-up voltage, board
power, USB CDC, and physical hardware are outside this evidence.
