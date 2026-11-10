# External components — NUCLEO-G491RE + BME280

The system descriptor attaches a BME280 device model at I2C1 address `0x77`.
Its modeled SDA/SCL route is PB9/PB8 respectively, matching Arduino D14/D15 on
the NUCLEO-G491RE. The smoke reads the modeled chip ID and raw temperature and
also checks controller pin activity and the board's PA5 LD2 output.

The attached sensor model does not test physical breakout wiring, pull-up
resistors, supply voltage, or electrical pad behavior.
