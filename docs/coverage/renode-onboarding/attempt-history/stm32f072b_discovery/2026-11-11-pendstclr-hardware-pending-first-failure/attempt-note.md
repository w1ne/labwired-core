# Failed first attempt: ICSR PENDSTCLR on a hardware-generated SysTick

- The exact imported target booted and executed 4,000,000 instructions. The firmware's new behavior checks all printed `1` and the run reached `MaxSteps`.
- The only failure was the ordered UART assertion: the formatter emitted `hardware first=first_wrap=1`, while the smoke expected `hardware first_wrap=1`.
- No model discrepancy was observed. The firmware label prefix will be corrected, then the same target will be rerun.
- Full metrics, UART, JUnit, result, snapshot, and build/run logs are preserved beside this note.
