# Feather M4 Express validation record

## Live vendor status checked 2026-11-03

- Adafruit's [Feather M4 Express product page](https://www.adafruit.com/product/3857)
  listed product 3857 at **$22.95**, with **Add to Cart**. The product page
  identifies the ATSAMD51J19A and its 512-KB flash / 192-KB RAM configuration.
- Microchip's live product page was read in real Chrome using
  `~/.venvs/mesh/bin/python ~/.hermes/scripts/mc-live-status.py ATSAMD51J19A`.
  The result was **In Production**. This is not a Wayback result.

## Board mapping and smoke scope

Adafruit's [Feather M4 pinout guide](https://learn.adafruit.com/adafruit-feather-m4-express-atsamd51/pinouts)
identifies D13 as the board's red LED. Adafruit's
[Feather M4 variant source](https://github.com/adafruit/ArduinoCore-samd/blob/master/variants/feather_m4/variant.cpp)
maps D13 to PORTA bit 23. The [board schematic](https://github.com/adafruit/Adafruit-Feather-M4-Express-PCB/blob/master/Adafruit%20Feather%20M4%20Express.sch)
connects D13/PA23 through R7 to the red LED and maps its return to ground.
The [Microchip SAM D5x/E5x datasheet, DS60001507](https://ww1.microchip.com/downloads/aemDocuments/documents/MCU32/ProductDocuments/DataSheets/SAM-D5x-E5x-Family-Data-Sheet-DS60001507.pdf)
documents the PORT register block.

The firmware writes PORTA DIRSET and OUTTGL and the smoke requires at least
eight edges on `porta:23`. This profile does not claim USB/CDC, SERCOM/UART,
QSPI flash, NeoPixel, charging, ADC/DAC, or other SAMD51 behavior.
