# Required Source Documents (Adafruit Metro M4)

## MCU Datasheet (authoritative)

1. Microchip SAM D5x / E5x Family Datasheet — **DS60001507** (memory map, MCLK/GCLK PCHCTRL, SERCOM USART, PORT):
   https://ww1.microchip.com/downloads/aemDocuments/documents/MCU32/ProductDocuments/DataSheets/SAM-D5x-E5x-Family-Data-Sheet-DS60001507.pdf

## Board Pinout / BSP

1. Adafruit Metro M4 Express pinout / product page:
   https://learn.adafruit.com/adafruit-metro-m4-express-featuring-atsamd51/pinouts
2. Adafruit ArduinoCore-samd Metro M4 variant (D13=PA16, Serial1=SERCOM3 on PA22/PA23):
   https://github.com/adafruit/ArduinoCore-samd/blob/master/variants/metro_m4/variant.cpp

## Address Cross-Check Only (not a source of truth)

1. CMSIS SAMD51J19A headers may be used to **cross-check** peripheral base addresses against DS60001507. Do not treat third-party simulator platform files as authoritative for LabWired models.
