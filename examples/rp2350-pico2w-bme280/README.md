# Raspberry Pi Pico 2 W + BME280

This demand-ranked board example uses the RP2350 profile on Raspberry Pi Pico 2 W
and reads an Adafruit BME280 over I2C0. RP2350/Pico boards rank #5 in the JetBrains
State of Embedded 2025 survey (12%). The board's built-in CYW43439 radio, Wi-Fi,
Bluetooth, and radio-controlled user LED are outside this profile.

## Wiring

| BME280 breakout | Pico 2 W |
| --- | --- |
| VIN | 3V3(OUT), physical pin 36 |
| GND | GND, physical pin 38 |
| SDA | GP4, physical pin 6 / I2C0 SDA |
| SCL | GP5, physical pin 7 / I2C0 SCL |

The tested Adafruit product 2652 breakout is configured for I2C address `0x77`.

## Copy-paste Arduino example

Select **Raspberry Pi Pico 2/RP2350** in the Earle Philhower Arduino-Pico core;
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
  Serial.println("PICO2W_BME280_OK");
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

The local smoke boots bare-metal Cortex-M33 firmware and checks BME280 ID `0x60`
and raw temperature `0x082DC0` through I2C0. Reproduce it using
[`VALIDATION.md`](VALIDATION.md); logs and simulator output are committed in
[`evidence/`](evidence/).

This profile intentionally leaves the CYW43439 out. It does not claim Wi-Fi,
Bluetooth, the WL_GPIO0 user LED, USB, radio/SPI routing, or electrical pad and
pull-up behavior. The RP2350 descriptor's POWERMAN and TBMAN windows remain
zero-answering stubs. UART0 output is captured for simulator diagnostics and
does not represent a physical USB or serial bridge.
