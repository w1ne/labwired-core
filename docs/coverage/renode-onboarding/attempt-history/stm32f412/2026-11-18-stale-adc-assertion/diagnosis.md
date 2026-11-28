# STM32F412 stale ADC smoke assertion (2026-11-18)

The exact imported system booted 800,000 instructions to `max_steps`. Six of
seven observable smoke checks passed; the assertion for the prior ADC1 single
conversion failed because it expected `adc1 in1=000003FF gated=1 clock=1
eoc=1 ok`, while the current shared firmware emitted
`adc1 in1=000003FF gated=1 clock=1 eoc=1 irq=1 clear=1 ok`. The numeric ADC
result, gate, clock, EOC, and new scan checks all passed. This is a stale UART
expectation left behind by the shared ADC1 smoke gaining interrupt checks, not
an ADC conversion or scan failure.

Preserved the run's metrics, build/run logs, UART, JUnit, result, and snapshot
here. Correct the expected UART text, then rerun only `stm32f412`.
