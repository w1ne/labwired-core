# Arduino Mega 2560 Rev3 vendor status

Checked live on 2026-11-03.

- **ATmega2560 lifecycle:** ran
  `~/.venvs/mesh/bin/python ~/.hermes/scripts/mc-live-status.py ATMEGA2560`.
  Real Chrome loaded <https://www.microchip.com/en-us/product/ATMEGA2560>; the
  live status returned **In Production**. No Wayback snapshot was used.
- **Board availability:** the [Arduino Official Store product page](https://store.arduino.cc/products/arduino-mega-2560-rev3)
  lists Arduino Mega 2560 Rev3, SKU A000067, at **$49.90** with **Add to cart**.
  The page identifies the board as ATmega2560-based and lists its 54 digital
  I/O pins, 16 analog inputs, and four UARTs.

The lifecycle check is for the MCU on the board; the store listing is for the
board itself.

## Re-check 2026-11-08 (Arduino matrix row)

- **ATmega2560 lifecycle:** `mc-live-status.py ATmega2560` in real Chrome
  returned **In Production** again.
- **Board availability:** the Arduino store answered HTTP 429 (rate limit) to
  the fetch. The availability above (2026-11-03) was not re-read today.
