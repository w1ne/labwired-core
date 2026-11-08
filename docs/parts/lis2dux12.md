# ST LIS2DUX12

The LIS2DUX12 is an active, volume-production 3-axis accelerometer. On
2026-11-04, ST's eStore listed LIS2DUX12TR as out of stock, while DigiKey listed
the active part with 1,193 units in stock. The dated source record and adapter
availability are in the [onboarding evidence](../../examples/nucleo-f407-lis2dux12/evidence/vendor-status.md).

## Modeled path

The model at `configs/devices/lis2dux12.yaml` currently covers:

- I²C identity (`WHO_AM_I = 0x47`) and the 0x18 address used with SA0 low.
- Main-page CTRL5 ODR and full-scale write/readback.
- A fixed 800 Hz data-ready interval while CTRL5 ODR bits are nonzero.
- Signed little-endian XYZ samples driven by `x`, `y`, and `z` input channels,
  scaled for the selected full scale.

The ODR cadence is a fixed approximation rather than a decode of every ODR
setting. FIFO, interrupts, temperature, I3C, electrical behavior, and the
embedded MLC/FSM/tap/free-fall/6D functions are not modeled. See the
[NUCLEO-F407 smoke](../../examples/nucleo-f407-lis2dux12/README.md) for the
firmware path and committed run evidence.

## Physical setup

The onboarding setup pairs a NUCLEO-F407 with ST's STEVAL-MKI238A DIL24 sensor
adapter. Strap CS high for I²C and SA0 low for address 0x18; connect I²C1 SCL/SDA
to PB6/PB7 and supply the adapter's VDD/VDDIO rails according to its schematic.
The [vendor receipt](../../examples/nucleo-f407-lis2dux12/evidence/vendor-status.md)
links the official datasheet, active product listing, and adapter.
