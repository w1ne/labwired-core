# External Components (Pico 2 / RP2350)

The base UART smoke uses on-chip peripherals only: clkrst (rp2350 profile),
UART0 (PL011), and SIO (LED GP25). The Pico 2 BME280 smoke adds the modeled
Bosch BME280 environmental sensor at I2C0 address `0x77`.

The BME280 device is attached directly to the RP2350 I2C0 controller model.
The system manifest records the board pin assignment (GP4 SDA, GP5 SCL), and
the firmware configures those pads for I2C0. The current RP2350 I2C model does
not reject a transaction when those pad muxes are wrong; pad electrical
routing, pull-ups, and voltage levels are therefore outside this evidence.

## Adding an external device

See [`examples/demo-blinky/`](../demo-blinky/README.md) for the
`external_devices` attach pattern. I2C/SPI sensors that attach on the
RP2040 lane attach here via the same `rp2040_i2c` / `rp2040_spi` models
on the RP2350 bases; check [`docs/boards/rp2350.md`](../../docs/boards/rp2350.md)
for the modeled-peripheral boundary (no TrustZone, Hazard3, HSTX, USB
enumeration, real bootrom).
