# L4 SPI sensor on the Arduino Nano Every

Stock ArduinoCore-megaavr `SPI` on SPI0 against the matrix MAX31855 kit.
`SPI.begin()` routes SPI0 to PORTMUX ALT2 (the Nano Every D11/D12/D13) and
sets SSD, ENABLE and MASTER; the sketch then drives CS (`SS` = D8 = PE3)
low and clocks four bytes with `SPI.transfer(0)`. It prints `LW_L4_OK` only
when the 32-bit frame is exactly 0x01901600 (thermocouple 25.0 °C, internal
22.0 °C, no fault), and `LW_L4_FAIL frame=0x...` otherwise.

`test.yaml` / `result.json` / `uart.log` / `junit.xml`: the matrix cell as
`run_matrix.py` ran it (it stops once `LW_L4_OK` is seen).

`full.yaml` / `full-result.json` / `full-uart.log`: the same firmware run
the full 500000 steps, asserting that the whole transcript is exactly
`LW_L4_BOOT` / `LW_L4_OK`.

`controls/`: the same firmware with the thermocouple at 100 °C. The sketch
prints `LW_L4_FAIL frame=0x06401600`, the frame a MAX31855 gives for 100 °C
and 22 °C, so the passing frame is read from the device, in byte order.

What this does not show: SPI0 does not observe CS (every attached device
sees every byte, and the MAX31855 kit frames on its soft-CS path), and
transfers are instant, so SCK rate and SPI mode are not exercised.

Paths in the yaml files are repo-relative records; the CLI resolves them
relative to the script, so run them from a copy with absolute paths.
