# NUCLEO-H533RE BME280 validation

The smoke builds `firmware-nucleo-h533re-bme280`, boots it on the focused
H533 profile, and reads BME280 register `0xD0` over I2C1. The firmware emits 16
LD2 high/low pulses only when the sensor returns its expected chip ID, `0x60`.
The smoke requires at least 32 observed PA5 transitions and records its
simulation output under `evidence/`.

I2C1 is the H5 I2C model; GPIOB exposes the board's PB6/PB7 Arduino route. No
zero-answering stub is used for I2C or the BME280. The test does not cover other
H533 memory banks, TrustZone, USB, the VCP, or physical supply behavior. It is
not a physical-board capture.

Reproduce with the command in [README.md](README.md). Current lifecycle and
availability evidence is in [`evidence/vendor-status.md`](evidence/vendor-status.md).
