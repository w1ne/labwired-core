# External components

The system manifest attaches a BME280 sensor model at I2C address `0x77`, routed
to SDA GPIO8 and SCL GPIO9 (J1 headers 9 and 10). GPIO7 is a board strapping
pin and is avoided. For a physical check, use a 3.3 V BME280 breakout and
connect its SDA, SCL, VIN, and GND to those pins and the board's 3V3/GND.
The DevKitC-1's addressable RGB LED on GPIO27 is intentionally outside this model.
For a separate GPIO5 bench check, place an external LED and current-limiting
resistor between J3 pin 9 (GPIO5) and ground.
