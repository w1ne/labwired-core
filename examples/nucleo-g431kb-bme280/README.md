# NUCLEO-G431KB + BME280

This example models ST's NUCLEO-G431KB (MB1430) with its exact STM32G431KBT6
package profile, the board's default user LED and ST-LINK VCP wiring, and an
external BME280 on I2C1.

## Demand rank

The MCU is in measured demand rank **#4**, “other STM32” (13%). STM32
Nucleo/Discovery boards are kit rank **#4** (23%). This fills an exact, active
Nucleo-32 board/package path while reusing the existing G431/G4 core models;
it adds no new MCU core or peripheral model.

## Wiring

| BME280 breakout | NUCLEO-G431KB |
| --- | --- |
| VIN | 3V3 |
| GND | GND |
| SDA | Nano A4 / PB7 |
| SCL | Nano A5 / PA15 |

The board's default solder bridges route I2C1 to PA15/PB7. LD2 is on PB8, and
the ST-LINK VCP is connected to USART2 on PA2/PA3. See [vendor-status.md](vendor-status.md)
for the dated live availability check.

## Copy-paste Arduino example

With the NUCLEO-G431KB selected in STM32duino and the Adafruit BME280 Library
installed, this reads the sensor ID and prints it over the board's USART2 VCP.
The board's default `Wire` pins are PA15/PB7.

```cpp
#include <Wire.h>
#include <Adafruit_BME280.h>

Adafruit_BME280 bme;

void setup() {
  Serial.begin(115200);
  Wire.setSCL(PA15);
  Wire.setSDA(PB7);
  Wire.begin();
  pinMode(PB8, OUTPUT); // NUCLEO-G431KB LD2

  if (!bme.begin(0x77, &Wire)) {
    Serial.println("BME280 not found");
    while (true) delay(10);
  }
  Serial.print("G431KB_BME280_OK id=0x");
  Serial.println(bme.sensorID(), HEX);
}

void loop() {
  digitalWrite(PB8, HIGH);
  delay(250);
  digitalWrite(PB8, LOW);
  delay(250);
}
```

## Simulator smoke

The committed bare-metal firmware reads BME280 `CHIP_ID` (`0xD0`) and the
three raw temperature registers over I2C1, prints the values on USART2, and
toggles LD2 only after the expected sensor response. See
[VALIDATION.md](VALIDATION.md) for the copy-paste smoke command and evidence.
The model covers the board-facing GPIO/I2C/UART subset named above; USB,
ST-LINK firmware, analog measurement, and physical electrical behavior remain
outside this profile.
