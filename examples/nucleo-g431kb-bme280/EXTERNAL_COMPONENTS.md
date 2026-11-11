# External component

| Part | Connection | Model | Exercised behavior |
| --- | --- | --- | --- |
| Adafruit BME280 STEMMA QT breakout, product 2652 | I2C1 at `0x77`; SCL PA15 / Nano A5, SDA PB7 / Nano A4 | [`configs/devices/bme280.yaml`](../../configs/devices/bme280.yaml) | Reads `CHIP_ID` (`0xD0`) and raw temperature (`0xFA`–`0xFC`). |

The NUCLEO-G431KB I/O is 3.3 V. Power the breakout from 3V3 and connect its
ground; the simulator does not model the breakout's supply or analog level
behavior.
