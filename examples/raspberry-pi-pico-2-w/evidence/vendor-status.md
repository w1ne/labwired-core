# Vendor status receipt — Raspberry Pi Pico 2 W

Checked live on 2026-11-03 for Raspberry Pi Pico 2 W (SC1633 / SC1634):

- Raspberry Pi's current [Pico 2 W product page](https://www.raspberrypi.com/products/raspberry-pi-pico-2/?variant=pico-2-w)
  says **“Available now from $5”** and **“Pico 2 series will remain in
  production until at least January 2040.”** The product portal lists board
  SKUs SC1633/SC1634. The 2040 commitment applies to the Pico 2 series.
- Raspberry Pi's current [Pico 2 W datasheet](https://pip-assets.raspberrypi.com/categories/1088-raspberry-pi-pico-2-w/documents/RP-008304-DS-3-pico-2-w-datasheet.pdf)
  identifies product codes SC1633/SC1634 and guarantees Pico 2 W availability
  until at least **January 2028**. This edition is dated 3 July 2026; the
  [product information portal](https://pip.raspberrypi.com/categories/1088-raspberry-pi-pico-2-w)
  lists the current document set.
- Raspberry Pi's current [Pico 2 W pinout](https://pip-assets.raspberrypi.com/categories/1088-raspberry-pi-pico-2-w/documents/RP-008305-DS-1-pico-2-w-pinout.pdf)
  identifies GP0 at physical header pin 1.
- The datasheet and product page identify the CYW43439 wireless interface.
  Infineon's [live CYW43439 product page](https://www.infineon.com/part/CYW43439)
  reports **“Active and preferred”** for CYW43439KUBGT. This radio is not
  modeled in this GP0-only profile.

The board vendor's live page says it is available now, and the exact product
datasheet gives an availability commitment beyond the check date. No third-
party stock quantity was used for this decision. The model claims only RP2350
GPIO behavior.
