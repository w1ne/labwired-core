# i.MX RT1064 ADC first attempt

The exact imported system booted and the modeled ADC completed calibration and
channel 0 conversion. The smoke failed only because its UART assertion expected
eight hex digits (`00000400`) while the firmware's four-digit 12-bit formatter
emitted `0400`. The metrics, UART, JUnit, result, snapshot, and build/run logs
are preserved here before the one-line assertion correction.
