# L3 I2C sensor on the Arduino Nano Every

Stock ArduinoCore-megaavr `Wire` on TWI0 against the matrix INA219 kit at
0x40. The sketch prints `LW_L3_OK` only after it reads the config register
back as 0x399F and the bus-voltage register as 0x19CA (two `requestFrom`
reads); a read with other values prints `LW_L3_FAIL bad_regs`, and a working
address phase with no data path prints `LW_L3_PARTIAL_NO_RX`.

`test.yaml` / `result.json` / `uart.log` / `junit.xml`: the matrix cell as
`run_matrix.py` ran it (it stops once `LW_L3_OK` is seen).

`full.yaml` / `full-result.json` / `full-uart.log`: the same firmware run
the full 500000 steps, asserting that the whole transcript is exactly
`LW_L3_BOOT` / `LW_L3_OK` — so neither the partial nor a failure line was
printed after the marker.

`controls/`: the same firmware with the INA219 strapped to 0x41. The address
phase at 0x40 is NACKed and the sketch prints `LW_L3_FAIL nack`.

Paths in the yaml files are repo-relative records; the CLI resolves them
relative to the script, so run them from a copy with absolute paths.
