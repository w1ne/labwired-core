# Raspberry Pi Pico W + BME280 vendor status

Checked live on 2026-11-17 UTC.

- Raspberry Pi's current [Pico W product information page](https://pip.raspberrypi.com/categories/686-raspberry-pi-pico-w) lists board SKU SC0918 and a datasheet updated 2026-07-15. Its [Pico W product brief](https://pip-assets.raspberrypi.com/categories/686-raspberry-pi-pico-w/documents/RP-008313-DS-1-pico-w-product-brief.pdf) states that the board will remain in production until at least January 2036. The brief identifies RP2040, dual Cortex-M0+ cores, 133 MHz, 264 KB SRAM, and the board's I2C/UART interfaces.
- Raspberry Pi's [Pico W pinout diagram](https://pip-assets.raspberrypi.com/categories/686-raspberry-pi-pico-w/documents/RP-008315-DS-1-PicoW-A4-Pinout.pdf) maps GP4/GP5 to the I2C header route and GP0/GP1 to the UART header route used by this example.
- Mouser's [exact SC0918 listing](https://www.mouser.com/en/ProductDetail/Raspberry-Pi/SC0918) showed 3,572 in stock, able to ship immediately, at $6.00. This is the Pico W board, not a substitute board.
- Adafruit's [BME280 STEMMA QT breakout, PID 2652](https://www.adafruit.com/product/2652) showed $14.95 and “In stock.” This smoke uses the assembled breakout at I2C address 0x77.

The simulator scope covers the RP2040, I2C0 on GP4/GP5, the BME280 chip-ID read, and UART0 on GP0. It does not model the CYW43439 Wi-Fi/Bluetooth path, the Pico W onboard LED's WL_GPIO0 link, USB, or board power behavior.
