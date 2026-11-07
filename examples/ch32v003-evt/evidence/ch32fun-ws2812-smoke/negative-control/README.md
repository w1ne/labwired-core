# Negative controls: ch32fun ws2812bdemo

Same firmware (`ch32fun/ws2812bdemo.elf`), same smoke script, same 6.2M
cycles. Each control removes one piece of this change.

## `no-spi-pad-map.log`: SPI1 publishes no pads

The chip yaml's `spi1` without `config: { pad_map: ch32v003 }`. The firmware
runs exactly as before: all 3 smoke checks pass (CNTR3 = 0, CIRC clear, EN
and TXDMAEN set), because DMA and SPI do the same register work. The strip
on PC6 decodes **0 LEDs**: the bit engine moves MOSI, but no pad carries it.
So the 190 LEDs in the real run come from the SPI1 MOSI wire reaching PC6,
not from anything else that toggles the pad.

## `no-cntr-rearm.log`: a CNTR store on an enabled channel only sets CNTR

`dma.rs` built with the CH32V003 CNTR re-arm arm disabled (`0x04 if false
&& ...`), so the store neither reloads the half/complete reference nor the
pointers, and no standing SPI1 TXE request is looked up. Result: 2/3 checks.
CFGR3 = 0x35b7, CIRC still set: the driver never reached the HT refill that
clears it. Only **6 LEDs** decode, the ones WS2812BDMAStart pre-filled.
Every later half-buffer refill depends on HT/TC at the CNTR the store loaded.

With the change: 3/3, and `colour-check/check-ws2812.log` matches all 190
LEDs the driver sends against the demo's own colour math (frame 9).
