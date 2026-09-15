# External Components (Waveshare RP2350-Zero)

No required external simulated components for minimal deterministic smoke.

Onboard parts (not extra modules):

| Part | Pad | Twin |
|------|-----|------|
| WS2812 RGB | GP16 (DIN) | `board_io` LED stand-in `led_gp16` on SIO pin 16. Not a NeoPixel kit. Timing/PIO of the real WS2812 is unproven here. |
| ME6217C33M5G LDO | 3V3 rail | Power is digital in the twin — not SPICE. |
| 12 MHz crystal | XIN/XOUT | Clock tree via `rp2350` clkrst profile. |
| 4 MB NOR flash | QSPI | XIP load of the ELF; not a flash programmer model. |
| BOOT, RUN | USB boot / reset | Not modeled as GPIOs. Bench: BOOT then RESET enters UF2. |

USB Type-C is the RP2350 USB peripheral. The twin does **not** enumerate USB. Do not attach a `usb_serial` bridge part — there is none on this board.

## Adding an external device

See [`examples/demo-blinky/`](../demo-blinky/README.md) for `external_devices`. I2C/SPI sensors that attach on the RP2040 lane attach here on the RP2350 bases. Header defaults (pico-sdk): I2C1 GP6/GP7, SPI1 GP10–13, UART0 GP0/GP1.
