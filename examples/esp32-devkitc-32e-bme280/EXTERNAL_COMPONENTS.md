# External components

The system manifest explicitly attaches one modeled `bme280` I2C device at
address `0x77`. On the ESP32-DevKitC-32E profile it is routed to GPIO21 (SDA)
and GPIO22 (SCL). The model reports the BME280 ID and deterministic raw sensor
registers used by the smoke. It does not model breakout pull-up resistance,
analog voltage levels, or physical sensor variation.
