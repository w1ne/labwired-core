# Raspberry Pi Pico 2 + Adafruit BME280 availability

Checked live on 2026-11-11.

- Raspberry Pi's current Pico 2 product page identifies it as an RP2350 board,
  says **Available now from $5**, and states the Pico 2 series remains in
  production until at least January 2040.
- Adafruit's BME280 STEMMA QT breakout (product 2652) page lists **In stock**
  at **$14.95** with an Add to Cart option.

Sources: [Raspberry Pi Pico 2 product page](https://www.raspberrypi.com/products/raspberry-pi-pico-2/),
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
