# Live vendor status — Waveshare RP2350-Zero

Checked 2026-11-06 in Google Chrome on Waveshare's official product page:

- Product: [Waveshare RP2350-Zero Mini Development Board](https://www.waveshare.com/product/rp2350-zero.htm)
- The page displayed **$4.79–$5.79** and an **Add to Cart** button.
- Both board variants were marked saleable in the rendered product data:
  SKU 28816 (without header) and SKU 29361 (with pre-soldered header), each
  `unsaleable: false`.
- The page identifies the board as an RP2350A board and lists an onboard
  WS2812. The [board wiki](https://www.waveshare.com/wiki/RP2350-Zero) and
  [schematic](https://files.waveshare.com/wiki/RP2350-Zero/RP2350_Zero.pdf)
  identify the WS2812 DIN connection as GPIO16.

The sales page was rendered by Chrome for this check because direct HTTP page
fetches returned 403. This records vendor availability for the board; it does
not claim physical firmware validation.
