# LIS2DUX12 on NUCLEO-F407

This example uses ST's LIS2DUX12 accelerometer on the STEVAL-MKI238A DIL24
adapter, connected to I2C1 of the NUCLEO-F407. For this setup, strap **CS high**
for I2C and **SA0 low** for address `0x18`; connect SDA/SCL to PB7/PB6, and
connect VDD and VDDIO to the NUCLEO-F407's 3.3 V rail with common ground, per
the [official
schematic](https://www.st.com/resource/en/schematic_pack/steval-mki238a-schematic.pdf).

The firmware reads `WHO_AM_I`, writes and reads back `CTRL5 = 0xB0` (800 Hz,
±2 g), polls `STATUS.DRDY`, then burst-reads signed little-endian XYZ data. It
prints the pass marker and turns on LD2 (PA5) only when the ID, configuration,
and expected sample all match. The input channels default to 0.061, -0.122, and
0.244 g, yielding 1000, -2000, and 4000 counts at the selected sensitivity.

The model covers that exercised main-page I2C path: identity, CTRL5, a fixed
800 Hz data-ready approximation, and live XYZ channels. It does not model FIFO,
interrupt outputs, temperature, I3C, embedded MLC/FSM/tap/free-fall/6D
functions, or electrical behavior. See [`vendor-status.md`](evidence/vendor-status.md)
for current availability and [`evidence/`](evidence/) for the committed smoke
build and run output.

Reproduce from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id lis2dux12-nucleo-f407 \
  --crate nucleo-f407-i2c \
  --target thumbv7em-none-eabi \
  --script examples/nucleo-f407-lis2dux12/onboarding-smoke.yaml \
  --system examples/nucleo-f407-lis2dux12/system.yaml \
  --out-dir examples/nucleo-f407-lis2dux12/evidence
```
