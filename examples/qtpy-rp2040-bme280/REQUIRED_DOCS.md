# Source documents

## QT Py RP2040 board

1. Adafruit QT Py RP2040 product and availability page (product 4900):
   https://www.adafruit.com/product/4900
2. Adafruit pinout guide (STEMMA QT on I2C1; GP22 SDA, GP23 SCL; UART1 TX on
   GP20):
   https://learn.adafruit.com/adafruit-qt-py-2040/pinouts
3. Raspberry Pi RP2040 datasheet (memory map, UART, I2C, clocks, reset):
   https://datasheets.raspberrypi.com/rp2040/rp2040-datasheet.pdf
4. NuttX QT Py RP2040 board reference (physical GPIO mapping, including
   STEMMA QT GP22/GP23 and UART1 TX GP20):
   https://nuttx.apache.org/docs/12.5.0/platforms/arm/rp2040/boards/adafruit-qt-py-rp2040/index.html

## BME280 sensor

1. Bosch BME280 datasheet (chip ID at `0xD0`, temperature at `0xFA..0xFC`):
   https://www.bosch-sensortec.com/media/boschsensortec/downloads/datasheets/bst-bme280-ds002.pdf
2. Adafruit BME280 STEMMA QT breakout, product 2652:
   https://www.adafruit.com/product/2652
