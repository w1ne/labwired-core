# External components

| Part | Connection | Model | Smoke coverage |
| --- | --- | --- | --- |
| Adafruit BME280 STEMMA QT (product 2652) | I²C0 at `0x77`; XIAO D4/GPIO22 SDA and D5/GPIO23 SCL | [`configs/devices/bme280.yaml`](../../configs/devices/bme280.yaml) | Reads `CHIP_ID` (`0xD0`) and raw temperature (`0xFA`–`0xFC`). |

The sensor is attached to the C6 I²C controller model. The pin route is from
Seeed's XIAO pin map. GPIO matrix pad function enforcement and electrical
levels are not part of this smoke.
