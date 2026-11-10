# External component

| Part | Connection | Model | Smoke coverage |
| --- | --- | --- | --- |
| Adafruit BME280 STEMMA QT (product 2652) | I²C0, address `0x77`, SDA GPIO6, SCL GPIO7 | [`configs/devices/bme280.yaml`](../../configs/devices/bme280.yaml) | Reads `CHIP_ID` (`0xD0`) and raw temperature (`0xFA`–`0xFC`). |

The LabWired board model assigns I²C0 SDA/SCL to GPIO6/GPIO7, and the smoke
configures that route explicitly. This pin-function mapping is model-only; no
silicon comparison is claimed. The test proves digital-twin register traffic
and the sensor response, not electrical levels, pull-up resistance, cable
behavior, or operation on a physical board.
