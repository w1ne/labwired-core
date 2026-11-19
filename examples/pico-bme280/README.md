# Raspberry Pi Pico + BME280

Run commands from the repository root.

Demand rank: **RP2040, rank 5** in the measured MCU list (12%); RP2040/RP2350
is demand group **#2**. This extends the already modelled Raspberry Pi Pico
board with a buyable I2C sensor and checks the complete board-to-sensor path.
Live stock evidence is in [`vendor-status.md`](vendor-status.md).

## Hardware and wiring

Use a standard Raspberry Pi Pico (RP2040) and the Adafruit BME280 STEMMA QT
breakout (PID 2652). Connect:

| Pico | BME280 breakout | Purpose |
| --- | --- | --- |
| 3V3(OUT) | VIN | breakout supply |
| GND | GND | common ground |
| GP4 (physical pin 6) | SDA | Pico I2C0 SDA |
| GP5 (physical pin 7) | SCL | Pico I2C0 SCL |

The breakout's default address is `0x77`; its board includes I2C pull-ups.
The Pico's onboard LED is GP25. The simulator system attaches the sensor model
to I2C0 and maps GP25 to its board LED. It does not simulate the cable,
analog pull-up voltage, sensor conversion timing, or environmental variation.

## Copy-paste Arduino example

Select **Raspberry Pi Pico** with the Arduino-Pico core, install no extra
libraries, and upload:

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t BME280_ADDRESS = 0x77;
constexpr uint8_t BME280_CHIP_ID_REGISTER = 0xD0;

void setup() {
  Serial.begin(115200);
  pinMode(LED_BUILTIN, OUTPUT);
  digitalWrite(LED_BUILTIN, LOW);

  pinMode(4, INPUT_PULLUP);  // GP4 / SDA
  pinMode(5, INPUT_PULLUP);  // GP5 / SCL
  Wire.begin();              // Arduino-Pico's default I2C0 route is GP4/GP5.

  Wire.beginTransmission(BME280_ADDRESS);
  Wire.write(BME280_CHIP_ID_REGISTER);
  if (Wire.endTransmission(false) != 0 ||
      Wire.requestFrom(BME280_ADDRESS, static_cast<uint8_t>(1)) != 1) {
    Serial.println("BME280 not found at 0x77");
    return;
  }

  if (Wire.read() != 0x60) {
    Serial.println("Unexpected BME280 chip ID");
    return;
  }

  digitalWrite(LED_BUILTIN, HIGH);
  Serial.println("PICO_BME280_OK ID=0x60 I2C0=GP4/GP5");
}

void loop() { delay(100); }
```

The LED turns on only after the sensor returns its datasheet chip ID (`0x60`)
from register `0xD0`.

## Build and simulate

```bash
python -m pip install platformio
examples/pico-bme280/build-firmware.sh
scripts/onboarding_smoke.sh \
  --target-id pico-bme280 \
  --script examples/pico-bme280/bme280-smoke.yaml \
  --system configs/systems/pico-bme280.yaml \
  --out-dir examples/pico-bme280/evidence/run \
  --firmware-build-script examples/pico-bme280/build-firmware.sh
```

The local smoke requires a GP25 LED edge. The fast-boot fixture turns the LED
on only after the I2C transaction returns `0x60`, so this assertion checks the
sensor read and board GPIO path together. The copy-paste Arduino sketch prints
the UART marker; the hosted oracle record is under
[`evidence/hosted-verification.json`](evidence/hosted-verification.json). The
committed local run record is under [`evidence/run/`](evidence/run/).

The model covers RP2040 I2C0 register transactions, I2C0 pin routing, the BME280
chip ID response, and the GP25 LED. The Arduino sketch also uses UART0 for its
console message. The local smoke does not claim UART capture, nor model the
breakout's voltage regulator or pull-up resistor values, sensor conversion
timing, or changing temperature, pressure, and humidity readings.
