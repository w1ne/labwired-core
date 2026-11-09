# Live availability check — Raspberry Pi Pico 2 (SC1631)

Checked live on 2026-11-09 UTC for the Pico 2 GP26/ADC0 smoke.

- Raspberry Pi's [Pico 2 product page](https://www.raspberrypi.com/products/raspberry-pi-pico-2/)
  names RP2350, says **“Available now from $5”**, and says the series will
  remain in production until at least January 2040.
- The exact manufacturer order code is **SC1631**. DigiKey's [SC1631 listing](https://www.digikey.com/en/products/detail/raspberry-pi/SC1631/24627136)
  identifies Raspberry Pi as manufacturer and marks the product **Active**.
  Its page showed 20,546 in stock and a $6.12 single-unit cut-tape price when
  checked; distributor inventory can change.
- Raspberry Pi's [Pico 2 datasheet](https://datasheets.raspberrypi.com/pico/pico-2-datasheet.pdf)
  documents the board pinout and availability guarantee. The Pico 2 board's
  GP29 is used internally for VSYS/3, so this profile only presents GP26–GP28
  as general-purpose analog inputs.

This board remains listed as available and in production. The ADC smoke uses a
simulated external potentiometer fixture; it is not a component fitted to the
Pico 2.
