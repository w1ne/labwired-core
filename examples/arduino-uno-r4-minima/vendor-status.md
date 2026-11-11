# Arduino UNO R4 Minima vendor status

Checked 2026-11-11 against the [Arduino Official Store product page](https://store.arduino.cc/products/uno-r4-minima): the page lists SKU **ABX00080**, displays a €15.37 sale price, and presents an **Add to cart** control. The same live page identifies the RA4M1 MCU, one I2C interface, 5 V board operation, and 48 MHz core clock. This is current product availability evidence; it does not establish stock in every region.

The BME280 smoke exercises the board's A4/SDA and A5/SCL interface in the simulator. On real hardware, use a BME280 breakout designed for a 5 V host, with appropriate level shifting and pull-ups; the UNO R4 Minima schematic notes its I2C pull-ups are not populated.

Pin mapping references: [Arduino UNO R4 Minima full pinout](https://docs.arduino.cc/5e218c7ed3622cbd09ab25357c345b84/ABX00080-full-pinout.pdf) and [board schematic](https://docs.arduino.cc/resources/schematics/ABX00080-schematics.pdf).
