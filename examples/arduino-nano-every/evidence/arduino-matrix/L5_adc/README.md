# L5 ADC on the Arduino Nano Every

`test.yaml` / `result.json` / `uart.log` / `junit.xml`: the matrix cell as
`run_matrix.py` ran it (stops once `LW_L5_OK` is seen, so the value line is
cut off).

`value.yaml` / `value-result.json` / `value-uart.log`: the same firmware run
the full 500000 steps, asserting `LW_L5_VAL 512 512` — stock `analogRead(A0)`
converts 2500 mV on AIN3 against the 5 V VDD reference to 512, twice.

`controls/`: the same firmware with A0 at 1250 mV reads 256 and fails.

Paths in the yaml files are repo-relative records; the CLI resolves them
relative to the script, so run them from a copy with absolute paths.
