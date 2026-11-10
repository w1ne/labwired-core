# External component

| Part | Connection | Model | Smoke coverage |
| --- | --- | --- | --- |
| Adafruit BME280 STEMMA QT (product 2652) | I²C0, address `0x77`, SDA GPIO4, SCL GPIO5 | [`configs/devices/bme280.yaml`](../../configs/devices/bme280.yaml) | Reads `CHIP_ID` (`0xD0`) and raw temperature (`0xFA`–`0xFC`). |

The C3 I²C0 model accepts the board's explicit GPIO4/GPIO5 route. The digital
smoke checks register traffic and sensor response; it does not claim electrical
levels, pull-up resistance, cable behavior, or operation on a physical board.
