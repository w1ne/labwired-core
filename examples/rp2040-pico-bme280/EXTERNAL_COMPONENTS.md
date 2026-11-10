# External Components (RP2040 Pico / BME280)

The Pico system profile attaches the Adafruit BME280 STEMMA QT sensor model to
RP2040 I2C0 at address `0x77`. The descriptor records GP4 as SDA and GP5 as
SCL, and the firmware selects those I2C0 pad functions. It also drives the
Pico's onboard active-high LED on GP25 through the RP2040 SIO model.

The smoke demonstrates the I2C controller/device transaction and the GPIO
output in the simulator. The model attaches the BME280 to I2C0 directly; it
does not verify physical pull-ups, voltage levels, wiring continuity, or the
electrical GP4/GP5 pad path. No physical Pico or sensor was tested.
