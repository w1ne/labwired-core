# Attempt 1 — CVR clear while SysTick is pending

- The imported STM32F072B-DISCO system booted and executed 4,000,000 instructions to `max_steps`.
- The new assertions all evaluated true in firmware: COUNTFLAG was set before the CVR write, the pending SysTick remained set after the write, COUNTFLAG cleared, and one handler ran after PRIMASK was restored. The UART output and summary register recorded those results.
- The host smoke failed because its `uart_ordered` assertion still expected the previous `systick cvr-clear` line. The measurement is preserved here as an assertion-format failure, not a simulator behavior failure.
- Evidence: `onboarding-metrics.json`, `simulation-output/uart.log`, `simulation-output/junit.xml`, `simulation-output/result.json`, and the run/build logs in this directory.
