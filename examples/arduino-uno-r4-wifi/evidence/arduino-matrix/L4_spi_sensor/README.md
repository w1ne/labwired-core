# L4 SPI sensor on the Arduino UNO R4 WiFi

Stock ArduinoCore-renesas `SPI` on SPI0 (`ra_spi`) against the matrix
MAX31855 kit. `SPI.begin()` muxes D11/D12/D13 (P411 MOSIA / P410 MISOA /
P102 RSPCKA) and programs SPI0 directly (`configSpi()`: SPCR = 0, MSTR,
SPDCR.SPBYT, SPDCR2.BYSW, SPCMD0 8-bit, SPE). The sketch then drives CS
(`SS` = D10 = P103) low and clocks four bytes with `SPI.transfer(0)`. Each
byte is `SPDR_BY = 0`, a spin on `SPSR.SPRF`, then an `SPDR_BY` load. The
sketch prints `LW_L4_OK` only when the 32-bit frame is exactly 0x01901600
(thermocouple 25.0 °C, internal 22.0 °C, no fault). Any other frame prints
`LW_L4_FAIL frame=0x...`.

`test.yaml` / `result.json` / `uart.log` / `junit.xml`: the matrix cell as
`run_matrix.py` ran it. It stops once `LW_L4_OK` is seen (153,803 steps).

`full.yaml` / `full-result.json` / `full-uart.log`: the same firmware run
for the full 5,000,000 steps. It asserts that the whole transcript is
exactly `LW_L4_BOOT` / `LW_L4_OK`.

`controls/`: the same firmware with the thermocouple at 100 °C. The sketch
prints `LW_L4_FAIL frame=0x06401600`.

What this does not show:
* SPI0 does not drive SSL or observe CS. Every attached device sees every
  frame, and the MAX31855 kit frames on its soft-CS path.
* Transfers complete at once, so the SCK rate and SPI mode are not
  exercised.
* The sketch uses byte frames only. The core's 32-bit `transfer(buf, n)`
  path (SPLW, SPB = 32, BYSW, two words queued on SPTEF) is covered by the
  `ra_spi` unit tests, not by firmware.

Paths in the yaml files are repo-relative records. The CLI resolves them
relative to the script, so run them from a copy with absolute paths.
