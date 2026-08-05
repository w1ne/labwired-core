# External Components (Pico 2 / RP2350)

No required external simulated components for minimal deterministic smoke.

The onboarding path uses on-chip peripherals only: clkrst (rp2350 profile),
UART0 (PL011), SIO (LED GP25).

## Adding an external device

See [`examples/demo-blinky/`](../demo-blinky/README.md) for the
`external_devices` attach pattern. I2C/SPI sensors that attach on the
RP2040 lane attach here via the same `rp2040_i2c` / `rp2040_spi` models
on the RP2350 bases; check [`docs/boards/rp2350.md`](../../docs/boards/rp2350.md)
for the modeled-peripheral boundary (no TrustZone, Hazard3, HSTX, USB
enumeration, real bootrom).
