# LabWired oracle verification

- Status: `model_verified`
- Target: `stm32l476`, with the `nucleo-l476rg` board component and BME280
  component at I²C address `0x77`.
- Firmware: Arduino with Adafruit BME280 Library 2.3.0; same source as
  `../src/main.ino`.
- Oracle: serial contains `BME280_OK` and
  `T=25.00C P=1013.25hPa H=50.00%RH`.
- Result: both clauses passed; stop reason was `assertions_passed`.

The model verifier reports two non-blocking approximations: it derives device
time from the configured 80 MHz CPU rate without tracking PLL changes, and it
assumes an ideal 3.3 V rail because no supply net is routed. Power-on and
brown-out behavior are not covered. Its circuit validator reported zero errors
and two `I2C_NO_PULLUP` warnings; this smoke uses the bus-level sensor model,
not an electrical simulation of the I²C pull-up network.
