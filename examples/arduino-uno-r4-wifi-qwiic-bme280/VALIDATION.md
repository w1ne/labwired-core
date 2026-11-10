# Validation — Arduino UNO R4 WiFi Qwiic BME280

The smoke builds a stock ArduinoCore-renesas sketch for the UNO R4 WiFi. It
starts `Wire1`, which the RA4M1 board variant maps to IIC0, then reads BME280
registers `0xD0` (chip ID), `0xF3` (status), and `0xFA..0xFC` (raw temperature).
The local CLI assertion accepts the success marker only after ID `0x60`, idle
status, and a nonzero temperature sample are observed.

The system profile maps Qwiic SDA=P401 and SCL=P400 and attaches the modeled
BME280 at address `0x77`. The firmware and descriptor do not claim analog or
electrical behavior of the cable or connector. This is simulator evidence, not
a physical-board test. The hosted verifier does not currently expose this
target, so no hosted `model_verified` claim is made.

The reused chip profile declares zero-answering stubs for MSTP, flash cache,
stack monitoring, and the reserved ADC unit 1 window. They are outside this
Qwiic sensor claim. The sensor smoke exercises the modeled IIC0 and BME280
blocks.

Run the documented PlatformIO build and `scripts/onboarding_smoke.sh` command
in the [README](README.md). The resulting logs and structured simulator result
are kept in `evidence/`.
