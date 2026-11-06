# Vendor status receipt — Raspberry Pi Pico 2 W

Checked live on 2026-11-06 for Raspberry Pi Pico 2 W (SC1633 / SC1634):

- Raspberry Pi's current [Pico 2 W product page](https://www.raspberrypi.com/products/raspberry-pi-pico-2/?variant=pico-2-w)
  still says **“Available now from $5”** and **“Pico 2 series will remain in
  production until at least January 2040.”** The 2040 commitment applies to
  the series, not specifically to the W board.
- Raspberry Pi's current [Pico 2 W datasheet](https://pip-assets.raspberrypi.com/categories/1088-raspberry-pi-pico-2-w/documents/RP-008304-DS-3-pico-2-w-datasheet.pdf)
  identifies product codes SC1633/SC1634 and guarantees Pico 2 W availability
  until at least **January 2028**. The current edition is dated 3 July 2026;
  the [product information portal](https://pip.raspberrypi.com/categories/1088-raspberry-pi-pico-2-w)
  lists it as updated 15 July 2026.
- Raspberry Pi's current [Pico 2 W pinout](https://pip-assets.raspberrypi.com/categories/1088-raspberry-pi-pico-2-w/documents/RP-008305-DS-1-pico-2-w-pinout.pdf)
  identifies GP0 at physical header pin 1 and the public header map used by
  the BME280 profile (GP4/GP5 at physical pins 6/7).
- The datasheet and product page identify the CYW43439 wireless interface.
  Infineon's [live CYW43439 product page](https://www.infineon.com/part/CYW43439)
  reports **“Active and preferred”** for CYW43439KUBGT. This radio is not
  modeled in this GP0-only profile.

The live vendor page lists the Pico 2 W as available now. The current exact-W
datasheet guarantees availability through January 2028; the broader series page
states the Pico 2 series production commitment through January 2040. No third-
party stock quantity was used for this decision. Individual board profiles
state their narrower modeled peripherals separately.
