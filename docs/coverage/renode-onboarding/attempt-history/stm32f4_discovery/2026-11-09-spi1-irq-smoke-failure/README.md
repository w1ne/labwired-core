# First SPI1 IRQ simulation attempt

The firmware built and the imported STM32F4-Discovery system executed 4,000,000 instructions to `max_steps`, but the new SPI assertions failed. Captured UART reported `SPI1 MAX31855=00000000 TXEIRQ=0 REARM=0 RXNEIRQ=0 RELEASED=1` and `SPI1 CHECKS=0`. The runner result, logs, JUnit, snapshot and metrics are retained here. The prior target pass was restored under `metrics/` while investigating; this attempt is not counted as a final pass.
