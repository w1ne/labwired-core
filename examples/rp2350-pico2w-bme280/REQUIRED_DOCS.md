# Hardware sources

- [Raspberry Pi Pico 2 W product page](https://www.raspberrypi.com/products/raspberry-pi-pico-2/?variant=pico-2-w): variant, current purchase path, and family production status.
- [Raspberry Pi Pico 2 W datasheet](https://pip-assets.raspberrypi.com/categories/1088-raspberry-pi-pico-2-w/documents/RP-008304-DS-2-pico-2-w-datasheet.pdf): RP2350 board, I2C-capable header, CYW43439 SPI connection, internal GPIO allocation, and WL_GPIO0 LED.
- [Infineon CYW43439 product page](https://www.infineon.com/part/CYW43439): current status of the radio IC omitted from this profile.
- [Adafruit BME280 product 2652](https://www.adafruit.com/product/2652): breakout used by the smoke design.

The system's GP4/GP5 I2C0 route follows the Pico 2 W header map. UART0 is only
used as a local simulator diagnostic capture.
