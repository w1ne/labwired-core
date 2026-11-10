# STM32H743 baseline before PSP STKALIGN coverage

Preserves the passing Round 527 Cortex-M7 exception-frame smoke before adding
the four-byte-aligned Thread/PSP case. The active metrics directory is replaced
by the target-only rerun; this snapshot keeps the prior UART, JUnit, result,
snapshot, and build/run logs available for comparison.
