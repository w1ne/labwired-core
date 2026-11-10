# Live availability — Raspberry Pi Pico 2 W and microSD breakout

Checked live on 2026-11-10 UTC.

- Raspberry Pi's [Pico 2 product page](https://www.raspberrypi.com/products/raspberry-pi-pico-2/)
  includes Pico 2 W as the wireless RP2350 variant, lists the series as available
  now, and says it will remain in production until at least January 2040.
- Adafruit's [Micro SD SPI/SDIO Card Breakout, PID 4682](https://www.adafruit.com/product/4682)
  listed **$3.50** and **In stock**. It is a 3 V-only module without level
  shifters, so this pairing uses the Pico 2 W's 3.3 V GPIO. The breakout does
  not include a microSD card.

Demand rank: **RP2040, rank 5** in the supplied JetBrains survey (12%). Pico 2 W
uses the current RP2350 successor identified in the demand brief as the new
flagship. This is a board/peripheral system extension, not a new MCU model.
The model uses a fixed 32 MiB read-only SDHC image with a FAT16
`/README.TXT`; it does not model custom card images or writes. Inventory and
prices can change after this check.
