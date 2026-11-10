# MIMXRT700-EVK stale pass bitmap assertion

The ten-million-instruction target run completed the expanded UART report and
all behavioral checks. The new MSP case set pass bit 10, producing the expected
firmware marker `pass=000007ff`; one memory assertion still expected the prior
`0x700003ff` bitmap. The smoke assertion is updated to the observed
`0x700007ff` value.

The full failed metrics, result, JUnit, UART, snapshot, and build/run logs are
preserved beside this note. This failed attempt is not counted as a passing
measurement.
