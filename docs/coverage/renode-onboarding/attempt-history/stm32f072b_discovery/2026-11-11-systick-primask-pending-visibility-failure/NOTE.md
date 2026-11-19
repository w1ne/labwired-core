# SysTick PRIMASK pending visibility failure

The exact imported `stm32f072b_discovery` system booted 4,000,000 instructions.
The smoke failed its new ordered UART assertion: the SysTick handler ran only
after PRIMASK was cleared, but ICSR did not report `PENDSTSET` while the
exception was pending. This is preserved as a model failure, not a firmware
boot failure. The generated target metrics and scoreboard record the failed
attempt; the preceding passing measurement is the committed repository state
before this change.
