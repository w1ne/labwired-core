# Raspberry Pi Pico W + BME280

This example covers the RP2040-based Raspberry Pi Pico W with an external
BME280 breakout. In the demand-ordered target list, RP2040 ranks #5 among MCUs
at 12%. This is a Pico W board-variant extension of existing RP2040 support,
not new silicon or a new vendor/core.

## Wiring

| BME280 breakout | Raspberry Pi Pico W |
| --- | --- |
| VIN | 3V3(OUT), pin 36 |
| GND | GND, pin 38 |
| SDA | GP4, pin 6 / I2C0 SDA |
| SCL | GP5, pin 7 / I2C0 SCL |

The modeled Adafruit 2652 breakout uses address `0x77` with SDO strapped high.

## Copy-paste Arduino example

Select **Raspberry Pi Pico W** in the Earle Philhower Arduino-Pico core; install
Adafruit BME280 Library and Adafruit Unified Sensor.

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
  Serial.println("PICOW_BME280_OK");
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

The bare-metal RP2040 smoke reads BME280 ID `0x60` and raw temperature
`0x082DC0` over modeled I2C0, then prints a success marker through UART0. It
does not test physical wiring or pad-level electrical behavior. See
[VALIDATION.md](VALIDATION.md) for the command and committed logs.

The profile intentionally excludes Pico W's CYW43439 Wi-Fi/Bluetooth radio and
its WL_GPIO0-driven onboard LED. It also does not exercise USB. The shared
RP2040 descriptor still lists an unexercised TBMAN stub; the smoke never reads
or writes that window. See [vendor status](vendor-status.md) for the live
product and stocking check.
