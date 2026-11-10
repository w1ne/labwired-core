# Reference documents used for this board profile

- Arduino [Nano Every documentation](https://docs.arduino.cc/hardware/nano-every)
  and [official store listing](https://store.arduino.cc/collections/nano-family/products/nano-every)
  identify the board and ATmega4809.
- Arduino's [Nano Every pinout PDF](https://docs.arduino.cc/resources/pinouts/ABX00028-full-pinout.pdf)
  and official [ArduinoCore-megaavr variant header](https://github.com/arduino/ArduinoCore-megaavr/blob/master/variants/nona4809/pins_arduino.h)
  map D20/SDA to PA2 and D21/SCL to PA3. The core's
  [board definition](https://github.com/arduino/ArduinoCore-megaavr/blob/master/boards.txt)
  selects ATmega4809 at 16 MHz.
- Microchip's [ATmega4808/4809 data sheet](https://ww1.microchip.com/downloads/en/DeviceDoc/ATmega4808-4809-Data-Sheet-DS40002173A.pdf)
  documents the TWI host block used by the descriptor.
- Microchip's [MCP9808 data sheet](https://ww1.microchip.com/downloads/en/DeviceDoc/MCP9808-0.5C-Maximum-Accuracy-Digital-Temperature-Sensor-Data-Sheet-DS20005095B.pdf)
  specifies TA at pointer `0x05`, manufacturer ID at `0x06`, device ID at
  `0x07`, and resolution at `0x08`.
- Adafruit's [MCP9808 breakout listing](https://www.adafruit.com/product/1782)
  documents the breakout used in the example.
