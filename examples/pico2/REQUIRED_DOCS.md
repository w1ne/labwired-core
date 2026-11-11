# Required Source Documents (Pico 2 / RP2350)

## MCU Reference + Register Maps

1. RP2350 Datasheet (memory map, peripherals, dual M33):
   https://datasheets.raspberrypi.com/rp2350/rp2350-datasheet.pdf
2. pico-sdk `hardware_regs` for RP2350 (addressmap.h, intctrl.h, sysinfo.h) —
   authoritative bases for the moved low-APB map (CLOCKS/RESETS/XOSC/PLLs/UART
   etc. differ from RP2040):
   https://github.com/raspberrypi/pico-sdk/tree/master/src/rp2350/hardware_regs
3. CHIP_ID = `0x30004927` (sysinfo.h / RP2350 datasheet SYSINFO).

## Board

1. Raspberry Pi Pico 2 product brief / pinout (GP25 user LED):
   https://www.raspberrypi.com/products/raspberry-pi-pico-2/
2. Official Pico-series pinout diagram (GP4 = I2C0 SDA, GP5 = I2C0 SCL):
   https://pip-assets.raspberrypi.com/categories/1005-raspberry-pi-pico-2/documents/RP-008301-DS-1-Pico-2-Pinout.pdf

## Sensor

1. Bosch BME280 datasheet, register map (`CHIP_ID` at `0xD0` and raw
   temperature at `0xFA..0xFC`):
   https://www.bosch-sensortec.com/media/boschsensortec/downloads/datasheets/bst-bme280-ds002.pdf
2. Adafruit BME280 STEMMA QT breakout product page and tutorial:
   https://www.adafruit.com/product/2652
