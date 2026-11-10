# Live availability evidence — Raspberry Pi Pico 2 W

Checked 2026-11-10.

- Raspberry Pi's [Product Information Portal entry](https://pip.raspberrypi.com/categories/1088-raspberry-pi-pico-2-w)
  identifies the product as **Raspberry Pi Pico 2 W**, lists SKUs **SC1633**
  and **SC1634**, and provides the current datasheet and schematic.
- The [official Pico 2 W datasheet](https://datasheets.raspberrypi.com/picow/pico-2-w-datasheet.pdf)
  says Raspberry Pi guarantees availability until at least **January 2028**.
  It specifies **2 MB** of on-board QSPI flash; the board descriptor and firmware
  linker map use that size.
- Mouser's [Raspberry Pi Pico 2 series listing](https://www.mouser.com/en/c/embedded-solutions/?m=Raspberry%20Pi&series=Pico%202)
  listed SC1633 as a new product at **US$7.00, 2,525 in stock**.
- Adafruit's [BME280 breakout, PID 2652](https://www.adafruit.com/product/2652)
  listed **US$14.95, in stock**. The [Adafruit guide](https://learn.adafruit.com/adafruit-bme280-humidity-barometric-pressure-temperature-sensor-breakout/overview)
  gives its default I2C address as `0x77`.

The stock count and price are time-sensitive. The profile intentionally omits
the CYW43439 wireless subsystem and the wireless-module-connected user LED.
