The target-only PRIMASK run booted and passed all new masking checks. It failed
two existing padded-frame xPSR assertions because the stacked APSR flags were
`0x21` for this firmware layout; the architectural T and STKALIGN bits matched.
The captured UART, JUnit, result, snapshot, metrics, and build/run logs are kept
here before making those assertions mask the architecturally defined bits.
