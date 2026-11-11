# External component

| Part | Connection | Model | Scope |
| --- | --- | --- | --- |
| Adafruit BME280 STEMMA QT (product 2652) | I2C0, address `0x77`, SDA A4/GPIO11, SCL A5/GPIO12 | `configs/devices/bme280.yaml` | Reads chip ID (`0xD0`) and raw temperature (`0xFA`–`0xFC`). |

The system manifest attaches the BME280 to I2C0 and records the selected board
route. The smoke does not model the breakout's pull-up resistors or analog
electrical behavior.
