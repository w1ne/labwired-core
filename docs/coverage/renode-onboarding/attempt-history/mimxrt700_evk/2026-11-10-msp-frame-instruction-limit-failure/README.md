# MIMXRT700-EVK MSP frame instruction limit

The imported Cortex-M33 run exercised the new misaligned Thread/MSP case and
reported `msp_context=0000001f`, the stacked R0 and xPSR, a zero padding word,
and restoration of the original MSP. The seven-million-instruction budget then
stopped the longer UART report before the final pass marker, so the overall
smoke result was correctly recorded as failed. The target smoke budget was
raised to let the expanded report finish.

The full failed metrics, result, JUnit, UART, snapshot, and build/run logs are
preserved beside this note. This attempt is not counted as a passing
measurement.
