# Superseded lazy FP smoke run

The target-only Round 531 run at 23:24 UTC booted the imported STM32H743 system
and executed 4,000,000 instructions, but failed the new lazy FP assertions.
UART showed LSPACT set at handler entry and still set after the handler when it
did not use FP; it cleared on exception return. The caller's FP registers were
restored. The assertions incorrectly computed the FP frame address from the
outer MSP even though the naked IRQ-pender first pushes r4/r7. The memory checks
therefore inspected the wrong 0x68-byte frame area.

Raw UART, JUnit, simulator result/snapshot, metric and runner logs are preserved
here. This result is one boot with failed smoke assertions, not a passing
platform measurement.
