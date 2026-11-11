# External components

| Component | Catalog type | Role in this profile |
| --- | --- | --- |
| Adafruit BME280 STEMMA QT breakout (PID 2652) | `bme280` | I2C1 sensor at address `0x77`; the smoke reads CHIP_ID and raw temperature registers. |

The simulator attaches the sensor to the I2C1 model. It does not model the
breakout's pull-ups, cable capacitance, supply, or board-level voltage behavior.
