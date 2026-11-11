# Adafruit QT Py RP2040 + BME280

The Adafruit QT Py RP2040 is a compact RP2040 board with a STEMMA QT I2C
connector. In the JetBrains State of Embedded 2025 survey cited by the
demand-ordered target brief, RP2040 ranks **#5 at 12%**; RP2040/RP2350 is the
brief's **#2 target group**. The exact QT Py RP2040 (Adafruit product 4900) is
listed in stock by Adafruit; see [live vendor status](vendor-status.md).

## Board profile and scope

This system models the RP2040, UART1 TX on the board's TX pad (GP20), and the
STEMMA QT connector's I2C1 bus on GP22/GP23. It attaches a BME280 model at
`0x77` and verifies the sensor ID and raw temperature. The physical USB-C
host/CDC path and addressable NeoPixel are not attached in this system profile,
so this smoke makes no claim about them. The shared RP2040 descriptor still
has TBMAN as an explicit storage stub; this firmware never accesses it.

| Function | Board connection | RP2040 signal | Smoke coverage |
| --- | --- | --- | --- |
| BME280 SDA | STEMMA QT | GP22 / I2C1 SDA | Reads ID and temperature |
| BME280 SCL | STEMMA QT | GP23 / I2C1 SCL | Reads ID and temperature |
| UART output | TX pad | GP20 / UART1 TX | Success marker captured by the simulator |
| USB host/CDC path | USB-C | USB | Not attached in this system profile |
| Addressable NeoPixel | On-board | GP12 data, GP11 power | Outside profile |

Connect an Adafruit BME280 STEMMA QT breakout (product 2652) to the board's
STEMMA QT socket with a STEMMA QT cable. The modeled breakout has SDO strapped
high, selecting address `0x77`.

## Copy-paste Arduino example

Select **Adafruit QT Py RP2040** in the Earle Philhower Arduino-Pico core and
install the Adafruit BME280 and Adafruit Unified Sensor libraries. Connect a
3.3 V USB-to-UART adapter's RX to the QT Py TX pad if you want the `Serial1`
output on hardware. `Wire1` is the STEMMA QT connector on this board.

```cpp
#include <Wire.h>
#include <Adafruit_BME280.h>

Adafruit_BME280 bme;

void setup() {
  Serial1.setTX(20);
  Serial1.begin(115200);
  Wire1.setSDA(22);
  Wire1.setSCL(23);
  Wire1.begin();

  if (!bme.begin(0x77, &Wire1)) {
    Serial1.println("BME280 not found");
    while (true) delay(10);
  }
  Serial1.println("QTPY_RP2040_BME280_OK");
}

void loop() {
  Serial1.print("Temperature C: ");
  Serial1.println(bme.readTemperature());
  Serial1.print("Pressure hPa: ");
  Serial1.println(bme.readPressure() / 100.0F);
  Serial1.print("Humidity %: ");
  Serial1.println(bme.readHumidity());
  delay(2000);
}
```

## Simulator smoke

The bare-metal smoke exercises I2C1 and UART1. It asserts BME280 ID `0x60`,
raw temperature `0x082DC0`, and the marker `QTPY_RP2040_BME280_OK`. Reproduce
the committed run with [VALIDATION.md](VALIDATION.md). This is simulator
evidence; it does not confirm physical board wiring or electrical behavior.
