# Superseded CPACR smoke assertion

The target-only STM32H743 run at 00:36 UTC booted the imported system and
executed 4,000,000 instructions, but failed the new CPACR UART assertion. The
smoke expected CP10/CP11 access to be clear when `main` began; captured UART
showed the firmware startup path had already enabled both fields (`0x00F00000`).
The simulator completed all existing exception and lazy-FP checks. The
assumption about the initial firmware state was wrong; this was not a CPACR
model failure.

Raw UART, JUnit, simulator result/snapshot, metric and runner logs are preserved
here. This is one boot with a failed assertion, not a passing platform
measurement.
