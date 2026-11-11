# Raspberry Pi Pico W status check

Checked 2026-11-11 UTC for board SC0918:

- Raspberry Pi's [Pico product page](https://www.raspberrypi.com/products/raspberry-pi-pico/)
  lists Pico W as available now for $6 and says the Pico 1 series remains in
  production until at least January 2036.
- Raspberry Pi's [Pico W product brief](https://pip-assets.raspberrypi.com/categories/686-raspberry-pi-pico-w/documents/RP-008313-DS-1-pico-w-product-brief)
  specifically states that Pico W remains in production until at least January
  2036.
- DigiKey's [SC0918 listing](https://www.digikey.com/en/products/detail/raspberry-pi/SC0918/16627943)
  showed product status **Active**, 6,268 in stock, and a one-piece cut-tape
  price of $7.65 during this check.
- Raspberry Pi's [Pico W documentation](https://www.raspberrypi.com/documentation/microcontrollers/pico-series.html)
  identifies Pico W as a first-generation RP2040 board and documents its
  separate wireless subsystem. The onboard LED is connected through
  `WL_GPIO0`, so this profile does not claim RP2040 GP25 LED support.

LabWired's Part Knowledge catalog did not have a Pico W / SC0918 entry. The
status and hardware facts above are therefore sourced from Raspberry Pi and the
distributor listing; the simulation record covers the shared RP2040 path.

This is a board-variant availability check, not a new silicon part selection.
