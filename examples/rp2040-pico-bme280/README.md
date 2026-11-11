# Raspberry Pi Pico + BME280

The Raspberry Pi Pico is an RP2040 board with its user LED on GP25. This
example attaches an Adafruit BME280 STEMMA QT breakout to I2C0 and demonstrates
a sensor identity read over GP4/GP5. In the JetBrains State of Embedded 2025
survey cited by the onboarding target brief, RP2040 ranks #5 among MCUs at 12%;
the RP2040/RP2350 target group is second in that ordered list.

## Wiring

| BME280 breakout | Raspberry Pi Pico |
| --- | --- |
| VIN | 3V3(OUT), pin 36 |
| GND | GND, pin 38 |
| SDA | GP4, pin 6 / I2C0 SDA |
| SCL | GP5, pin 7 / I2C0 SCL |

The tested breakout is Adafruit product 2652, whose SDO strap selects I2C
address `0x77`.

## Copy-paste Arduino example

Select **Raspberry Pi Pico/RP2040** in the Earle Philhower Arduino-Pico core;
install Adafruit BME280 Library and Adafruit Unified Sensor.

```cpp
#include <Wire.h>
#include <Adafruit_BME280.h>

Adafruit_BME280 bme;

void setup() {
  Serial.begin(115200);
  Wire.setSDA(4);
  Wire.setSCL(5);
  Wire.begin();
  if (!bme.begin(0x77, &Wire)) {
    Serial.println("BME280 not found");
    while (true) delay(10);
  }
  Serial.println("PICO_BME280_OK");
}

void loop() {
  Serial.print("Temperature C: ");
  Serial.println(bme.readTemperature());
  Serial.print("Pressure hPa: ");
  Serial.println(bme.readPressure() / 100.0F);
  Serial.print("Humidity %: ");
  Serial.println(bme.readHumidity());
  delay(2000);
}
```

## Simulator smoke

The local smoke compiles a Cortex-M0+ firmware image and asserts BME280 ID
`0x60`, raw temperature `0x082DC0`, and GP25 LED transitions. Reproduce it
using [VALIDATION.md](VALIDATION.md); the logs and result are under
[`evidence/`](evidence/). These results are simulator-only. The attached
sensor model does not prove physical wiring or pad-level electrical behavior.

The RP2040 chip descriptor already existed; this adds an end-to-end Pico
customer design smoke rather than a new silicon model. See the shared
[RP2040 board coverage](../../docs/boards/rp2040.md) for unexercised blocks.
