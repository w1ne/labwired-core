# NUCLEO-G491RE + BME280

This profile adds an Adafruit BME280 STEMMA QT breakout (product 2652) to the
existing ST NUCLEO-G491RE board model. It exercises the G491RE's I2C1 controller
on PB8/PB9 and the board's LD2 LED on PA5.

## Demand rank

This is a peripheral coverage expansion on an existing board, not a new MCU or
board pick. The STM32G491RE falls under demand rank **#4**, “other STM32” (13%);
Nucleo/Discovery boards are also kit rank **#4** (23%). The measured leaders
remain ESP32, ATmega328, STM32F103, and other STM32. Availability notes are in
[vendor-status.md](vendor-status.md).

## Wiring

| BME280 breakout | NUCLEO-G491RE |
| --- | --- |
| VIN | 3V3 |
| GND | GND |
| SDA | Arduino D14 / PB9 (I2C1 SDA) |
| SCL | Arduino D15 / PB8 (I2C1 SCL) |

The Adafruit breakout uses address `0x77` when its SDO strap is high, as modeled
here. ST's UM2505 documents the D14/D15 pin mapping.

## Copy-paste Arduino example

Select the NUCLEO-G491RE in the STM32duino board package and install the Adafruit
BME280 Library plus Adafruit Unified Sensor. This basic example initializes the
sensor, prints readings over the board's configured serial port, and blinks LD2
after a successful sensor read.

```cpp
#include <Wire.h>
#include <Adafruit_BME280.h>

Adafruit_BME280 bme;

void setup() {
  Serial.begin(115200);
  Wire.setSDA(PB9);
  Wire.setSCL(PB8);
  Wire.begin();
  Wire.setClock(100000);
  pinMode(PA5, OUTPUT);

  if (!bme.begin(0x77, &Wire)) {
    Serial.println("BME280 not found");
    while (true) delay(10);
  }
  Serial.println("G491_BME280_OK");
}

void loop() {
  Serial.print("Temperature C: ");
  Serial.println(bme.readTemperature());
  Serial.print("Pressure hPa: ");
  Serial.println(bme.readPressure() / 100.0F);
  Serial.print("Humidity %: ");
  Serial.println(bme.readHumidity());
  digitalWrite(PA5, HIGH);
  delay(250);
  digitalWrite(PA5, LOW);
  delay(1750);
}
```

## Simulator smoke

The committed bare-metal smoke reads BME280 ID `0x60` and raw temperature
`0x082DC0` over I2C1, checks the bus traffic and PB8/PB9 activity, then requires
LD2 transitions. Reproduce it with [VALIDATION.md](VALIDATION.md); logs and
structured output are under [`evidence/`](evidence/). This is local simulator
evidence, not a physical board or electrical validation.
