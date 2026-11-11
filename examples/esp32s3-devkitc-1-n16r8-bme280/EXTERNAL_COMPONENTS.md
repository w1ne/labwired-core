# External component

| Part | Connection | Model | Scope |
| --- | --- | --- | --- |
| Adafruit BME280 STEMMA QT (product 2652) | I2C0, address `0x77`, SDA GPIO8, SCL GPIO9 | `configs/devices/bme280.yaml` | The smoke reads `CHIP_ID` (`0xD0`) and raw temperature (`0xFA`–`0xFC`). |

The sensor model is attached to the I2C0 controller in the system manifest. The
manifest records the board's pin route; this smoke does not test analog
electrical behavior, pull-up resistance, or the physical board.
