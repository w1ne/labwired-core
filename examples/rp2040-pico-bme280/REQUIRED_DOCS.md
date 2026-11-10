# Source Documents (Raspberry Pi Pico / RP2040)

## MCU and register map

1. Raspberry Pi RP2040 datasheet (memory map, I2C0, SIO, clocks and resets):
   https://datasheets.raspberrypi.com/rp2040/rp2040-datasheet.pdf
2. Raspberry Pi Pico C/C++ SDK register headers (`rp2040/hardware_regs`):
   https://github.com/raspberrypi/pico-sdk/tree/master/src/rp2040/hardware_regs

## Board

1. Raspberry Pi Pico product and availability page:
   https://www.raspberrypi.com/products/raspberry-pi-pico/
2. Raspberry Pi Pico datasheet and pinout, including GP4/GP5 and GP25:
   https://pip.raspberrypi.com/categories/610-raspberry-pi-pico

## Sensor

1. Bosch BME280 datasheet (CHIP_ID at `0xD0`, temperature output at `0xFA..0xFC`):
   https://www.bosch-sensortec.com/media/boschsensortec/downloads/datasheets/bst-bme280-ds002.pdf
2. Adafruit BME280 STEMMA QT breakout, product 2652:
   https://www.adafruit.com/product/2652
