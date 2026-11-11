# External components

| Part | Connection | Model | Smoke coverage |
| --- | --- | --- | --- |
| Adafruit BME280 STEMMA QT breakout (product 2652) | I²C0 at `0x77`; Feather SDA GPIO19 and SCL GPIO18 | [`configs/devices/bme280.yaml`](../../configs/devices/bme280.yaml) | Reads chip ID (`0xD0`) and raw temperature (`0xFA`–`0xFC`). |

The Adafruit guide assigns GPIO19 to SDA, GPIO18 to SCL and GPIO20 to the
STEMMA QT power switch. GPIO20 is raised by firmware. The simulator currently
does not connect that power switch electrically to the external sensor. The
board's built-in MAX17048 and AHT20 are not included in this narrow smoke
profile because neither is exercised here.
