# External components

The system manifest attaches one modeled BME280 sensor to the STM32F103C8T6's
I2C1 peripheral at address `0x77`. It represents the buyable Adafruit BME280
STEMMA QT breakout, PID 2652. The firmware probes register `0xD0` and requires
the BME280 ID byte `0x60`.

The virtual attachment proves the sensor-register transaction against the
engine's BME280 model. It does not include a physical cable, breakout PCB,
pull-ups, or electrical measurements.
