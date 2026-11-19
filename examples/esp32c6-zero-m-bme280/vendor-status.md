# Waveshare ESP32-C6-Zero-M vendor status

Checked live on 2026-11-18 UTC.

Waveshare's current [ESP32-C6-Zero product page](https://docs.waveshare.com/ESP32-C6-Zero)
lists the header version as **ESP32-C6-Zero-M**, manufacturer SKU **26976**, and
links its pinout and schematic. The current manufacturer page describes an
ESP32-C6FH8 with 8 MB flash and an onboard WS2812 LED.
The manufacturer provides an [official store listing for SKU 26976](https://www.waveshare.com/esp32-c6-zero.htm?sku=26976).

Electrokit's live [ESP32-C6-Zero-M listing](https://www.electrokit.com/en/esp32-c6-mini-development-board-with-headers)
identified the exact manufacturer part number and listed **232 units in
stock** at **115 SEK including VAT**, with an Add to cart control and an
estimated 1–3 weekday delivery.

The two live pages disagree on the flash variant: Waveshare's current page says
C6FH8 / 8 MB, while Electrokit's stocked listing says C6FH4 / 4 MB. This board
profile uses the 4 MB `esp32c6fh4` descriptor to match the listed stock and to
avoid claiming the additional 4 MB. The committed smoke image fits within 4 MB.
This profile does not resolve which flash variant Electrokit ships if its stock
changes.

The product remains in Waveshare's current catalog and is listed in stock by
the distributor above. Neither live page marks it NRND or obsolete.
