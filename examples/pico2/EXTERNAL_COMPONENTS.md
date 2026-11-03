# External components (Pico 2 / RP2350)

The minimal deterministic smoke requires no external simulated components.
The narrow board slice uses UART0 (PL011), IO_BANK0 function selection, and
SIO for the onboard GP25 LED output.

## Adding an external device

See [`examples/demo-blinky/`](../demo-blinky/README.md) for the
`external_devices` attach pattern. I2C/SPI devices can use the shared RP2040
models at their RP2350 addresses when the selected system profile includes
those peripherals. See [`docs/boards/rp2350.md`](../../docs/boards/rp2350.md)
for the broader chip profile's modeled-peripheral boundary.
