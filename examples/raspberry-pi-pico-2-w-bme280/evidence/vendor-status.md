# Live vendor status — Raspberry Pi Pico 2 W + BME280

Checked live on 2026-11-06 UTC:

- Raspberry Pi's [Pico 2 W product page](https://www.raspberrypi.com/products/raspberry-pi-pico-2/?variant=pico-2-w)
  says **“Available now from $5”** and that the **Pico 2 series** will remain
  in production until at least January 2040. This is a family commitment, not
  an exact-W commitment.
- Raspberry Pi's [Pico 2 W product portal](https://pip.raspberrypi.com/categories/1088-raspberry-pi-pico-2-w)
  lists the board SKUs SC1633 and SC1634 and the current datasheet edition.
  The exact-W [datasheet](https://pip-assets.raspberrypi.com/categories/1088-raspberry-pi-pico-2-w/documents/RP-008304-DS-3-pico-2-w-datasheet.pdf)
  guarantees Pico 2 W availability until at least January 2028.
- Adafruit's [BME280 I²C breakout listing](https://www.adafruit.com/product/2652)
  identifies product ID 2652 and showed **In stock** at **$14.95**, with an
  Add to Cart button.
- Adafruit's [BME280 pinout guide](https://learn.adafruit.com/adafruit-bme280-humidity-barometric-pressure-temperature-sensor-breakout/pinouts)
  gives its default I²C address as `0x77` (or `0x76` when the address jumper is changed).
- Raspberry Pi's current [Pico 2 W pinout](https://pip-assets.raspberrypi.com/categories/1088-raspberry-pi-pico-2-w/documents/RP-008305-DS-1-pico-2-w-pinout.pdf)
  is the source for the profile's GP4/GP5 header route. The separate board
  [datasheet](https://pip-assets.raspberrypi.com/categories/1088-raspberry-pi-pico-2-w/documents/RP-008304-DS-3-pico-2-w-datasheet.pdf)
  identifies RP2350, the I²C controllers, and the CYW43439 wireless module.

The board and sensor were both listed as buyable by their vendors at the check
time. The system profile models RP2350 Cortex-M33, I²C0, and BME280; it omits
the CYW43439 radio, USB, and the wireless-controlled LED.
