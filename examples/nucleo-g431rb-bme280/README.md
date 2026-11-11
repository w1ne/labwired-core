# NUCLEO-G431RB + BME280

This example profiles ST's **NUCLEO-G431RB (MB1367)** with its exact
STM32G431RBT6 model, the default user LED and ST-LINK VCP, and an external
BME280 on the board's default I2C1 pins.

## Demand rank

The MCU falls under demand rank **#4**, “other STM32” (13%). STM32
Nucleo/Discovery boards are kit rank **#4** (23%). This adds the missing
Nucleo-64 board wiring and smoke for an already-modelled G431RB. It reuses the
G4 core/peripheral models; it does not add a new MCU core. See
[vendor-status.md](vendor-status.md) for the dated active-status and stock check.

## Wiring

| Adafruit BME280 breakout | NUCLEO-G431RB |
| --- | --- |
| VIN | 3V3 |
| GND | GND |
| SDA | Arduino D14 / PB9 (I2C1 SDA) |
| SCL | Arduino D15 / PB8 (I2C1 SCL) |

The profile uses the board's default I2C1 pin mapping and BME280 address
`0x77`. LD2 is PA5. The default ST-LINK virtual COM port is LPUART1 on PA2/PA3.

## Copy-paste Arduino example

Select **NUCLEO-G431RB** in the STM32duino board package and install the Adafruit
BME280 Library and Adafruit Unified Sensor dependency:

```cpp
#include <Wire.h>
#include <Adafruit_BME280.h>

Adafruit_BME280 bme;

void setup() {
  Serial.begin(115200); // default ST-LINK VCP on LPUART1
  Wire.setSDA(PB9);
  Wire.setSCL(PB8);
  Wire.begin();
  Wire.setClock(100000);
  pinMode(PA5, OUTPUT); // LD2

  if (!bme.begin(0x77, &Wire)) {
    Serial.println("BME280 not found");
    while (true) delay(10);
  }
  Serial.println("G431RB_BME280_OK");
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

The bare-metal fixture reads BME280 `CHIP_ID` (`0x60`) and raw temperature
(`0x082DC0`) over I2C1, checks the I2C trace, then toggles LD2 and reports on
LPUART1. Reproduce it with [VALIDATION.md](VALIDATION.md). Logs and structured
output are committed under [`evidence/`](evidence/).

The profile directly attaches the sensor to the I2C controller model. The
simulator does not enforce the PB8/PB9 pad route electrically or simulate
pull-ups and voltage levels; ST-LINK hardware, ADC/motor-control blocks, and
the other board connectors are outside this smoke. No physical board was tested.
