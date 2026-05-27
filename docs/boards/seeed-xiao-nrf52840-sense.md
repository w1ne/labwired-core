# Seeed XIAO nRF52840 Sense

The Seeed XIAO nRF52840 Sense is a compact nRF52840 board with USB, RGB LED,
IMU, microphone, and exposed XIAO castellated pins.

## Status

| Aspect | Status |
|--------|--------|
| Chip yaml | `configs/chips/nrf52840.yaml` |
| System yaml | `configs/systems/seeed-xiao-nrf52840-sense.yaml` |
| Example | `examples/seeed-xiao-nrf52840-sense/` |
| Firmware | `crates/firmware-nrf52840-demo/` |
| Tier | smoke: UART0 + GPIO + SPIM0 register task path |

## Modeled Peripherals

| Peripheral | Base | Notes |
|------------|------|-------|
| UART0 | `0x40002000` | Nordic UART TXD smoke path |
| SPIM0 | `0x40003000` | Register/task/event smoke model |
| GPIO0 | `0x50000000` | nRF OUT/OUTSET/OUTCLR/DIR task register subset |
| GPIO1 | `0x50001000` | Synthetic non-overlapping LabWired mapping for P1 board I/O |

## Validation Boundary

This target has automated LabWired simulation coverage for UART/GPIO/SPIM0.
The connected USB board was detected locally and bootloader entry was verified,
but no SWD probe was available, so hardware register-dump parity is not claimed
yet.
