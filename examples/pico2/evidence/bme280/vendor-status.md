# Raspberry Pi Pico 2 + Adafruit BME280 availability

Checked live on 2026-11-10 at 17:13 UTC.

- Raspberry Pi's current Pico 2 product page says the RP2350 board is available
  now from $5 and states that the Pico 2 series will remain in production until
  at least January 2040.
- DigiKey's exact Raspberry Pi product `SC1631` page identifies it as the
  Raspberry Pi Pico 2 / RP2350 board, lists **Part Status: Active**, **19,421
  in stock**, a **$6.12** single-unit price, and an Add to Cart option.
- Adafruit's BME280 STEMMA QT breakout (product 2652) page showed **In stock**
  at **$14.95** with an Add to Cart option.

Sources: [Raspberry Pi Pico 2 product page](https://www.raspberrypi.com/products/raspberry-pi-pico-2/),
[DigiKey SC1631 product listing](https://www.digikey.com/en/products/detail/raspberry-pi/SC1631/24627136),
and [Adafruit BME280 product 2652](https://www.adafruit.com/product/2652).

## Verification scope

The local LabWired CLI smoke runs Cortex-M33 firmware on the RP2350 model,
reads BME280 `CHIP_ID` (`0x60`) and raw-temperature sample (`0x082DC0`) through
I2C0, then toggles the onboard LED on GP25. UART text carries diagnostic values
in the simulator, but this bare-metal smoke does not configure a physical UART.
The sensor is attached directly to the I2C controller model; the RP2350 model
does not electrically enforce GP4/GP5 pad routing or simulate pull-ups and
voltage levels. No physical Pico 2 was tested. The profile does not claim
wireless, USB, TrustZone, Hazard3, second core, or hardware parity.
