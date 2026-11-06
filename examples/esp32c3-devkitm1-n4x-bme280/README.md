# ESP32-C3-DevKitM-1-N4X + BME280

**Demand rank:** ESP32 ranks first in the 2025 JetBrains State of Embedded MCU
survey (38%). This example extends the current N4X board profile with a common
environmental-sensing design: an Adafruit BME280 breakout on I2C0.

The narrow descriptor exposes the C3 GPIO matrix, I2C0 controller, UART0, and
the attached BME280 model. It omits radio, USB, RGB LED, buttons, and other
board functions. The firmware performs repeated-start I2C register reads and
prints the BME280 chip ID plus raw temperature ADC value. The smoke asserts
both values, so an unattached sensor or a broken route fails.
The model's default environmental inputs are 25 °C, 1013.25 hPa, and 50% RH;
the smoke checks the raw temperature ADC bytes rather than claiming a physical
measurement or compensated accuracy.

## Wiring

Connect the Adafruit BME280 breakout to the ESP32-C3-DevKitM-1-N4X:

| BME280 | ESP32-C3-DevKitM-1-N4X |
|---|---|
| VIN | 3V3 |
| GND | GND |
| SDA | GPIO4 (J3 pin 11) |
| SCK/SCL | GPIO5 (J3 pin 10) |

The breakout's default I2C address is `0x77`. The N4X board is the active SKU;
the older unsuffixed DevKitM-1 variant is marked EOL. Current availability
evidence is in [`evidence/vendor-status.md`](evidence/vendor-status.md).

## Copy-paste Arduino example

Install the Adafruit BME280 and Adafruit Unified Sensor libraries, then upload:

```cpp
#include <Wire.h>
#include <Adafruit_Sensor.h>
#include <Adafruit_BME280.h>

Adafruit_BME280 bme;

void setup() {
  Serial.begin(115200);
  Wire.begin(4, 5);  // SDA=GPIO4, SCL=GPIO5

  if (!bme.begin(0x77, &Wire)) {
    Serial.println("BME280 not found");
    while (true) delay(10);
  }

  Serial.print("Temperature C: ");
  Serial.println(bme.readTemperature());
  Serial.print("Pressure hPa: ");
  Serial.println(bme.readPressure() / 100.0F);
  Serial.print("Humidity %: ");
  Serial.println(bme.readHumidity());
}

void loop() {}
```

## Run the repository smoke

From the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id esp32c3-devkitm1-n4x-bme280 \
  --crate firmware-esp32c3-bme280-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/esp32c3-devkitm1-n4x-bme280/onboarding-smoke.yaml \
  --system configs/systems/esp32c3-devkitm1-n4x-bme280.yaml \
  --out-dir examples/esp32c3-devkitm1-n4x-bme280/evidence
```

Committed evidence records the firmware build and local simulator assertions.
This is model evidence, not a physical-board test.
