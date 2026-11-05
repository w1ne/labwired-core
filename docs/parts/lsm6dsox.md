# ST LSM6DSOX

ST currently labels the LSM6DSOX **Active** and in volume production. On
2026-11-05 Newark offered LSM6DSOXTR cut tape for order at quantity one, but
listed delivery starting 2026-11-27 rather than stock on hand; see the dated
[vendor receipt](../../examples/nucleo-f767zi-lsm6dsox/evidence/vendor-status.md).

## Modeled path

The [LSM6DSOX configuration](../../configs/devices/lsm6dsox.yaml) is a partial
I2C model that supports:

- `WHO_AM_I` at `0x0F`, returning `0x6C`.
- `CTRL1_XL` read/write and a fixed 416 Hz data-ready approximation when its
  ODR field is nonzero.
- Signed little-endian XYZ accelerometer samples at `0x28`–`0x2D`, scaled by
  the selected accelerometer full scale.

The gyro, temperature sensor, FIFO, interrupt and embedded-function engines,
SPI/I3C interfaces, electrical behavior, and other ODR timing choices are not
modeled. The [NUCLEO-F767ZI smoke](../../examples/nucleo-f767zi-lsm6dsox/README.md)
demonstrates the supported path and committed local simulation evidence.
