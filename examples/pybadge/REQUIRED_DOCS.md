# PyBadge source references

| Source | Used for |
| --- | --- |
| [Adafruit PyBadge product page, product 4200](https://www.adafruit.com/product/4200) | Product identity, ATSAMD51J19, display and onboard component inventory, live purchase status |
| Adafruit ArduinoCore-samd `variants/pybadge_m4/variant.cpp` and `variant.h` | D13 / `LED_BUILTIN` mapped to PA23 |
| Microsoft `uf2-samdx1` `boards/arcade_pybadge/board_config.h` | CF2 pin table and 16 KiB bootloader/application offset |
| Microchip ATSAMD51J19A datasheet DS60001507 | Flash/RAM layout and SAM D5x register definitions |
| Microchip [ATSAMD51J19A product page](https://www.microchip.com/en-us/product/atsamd51j19a) | Live lifecycle status, checked with the browser-based Microchip status script |

The check date and vendor responses are recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).
