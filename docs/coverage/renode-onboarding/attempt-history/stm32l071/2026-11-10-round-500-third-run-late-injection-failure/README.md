# STM32L071 Round 500 third run

This attempt delayed the five UART injections until simulated cycle 3,120,000,
after the SPI RXNE IRQ checks. The new RX checks still all passed, but SPI1 and
SPI2 continued to report `pend=0`. This preserves the scheduled-injection
attempt and confirms the failure occurs before those injections are delivered.
