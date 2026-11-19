# STM32F103 SPI1 first attempt: stimulus arrived after the read

The first `--only stm32f103` run booted 50,000,000 instructions and passed 55
of 56 assertions. SPI1 TXE and RXNE IRQ assertion/release checks passed, but the
MAX31855 returned its documented power-on frame `0x01901600` (25 °C) before the
50 °C `after_cycles` stimulus was applied. The later stimulus cannot affect
that already completed transaction.

The next run removes the unnecessary temperature stimulus and checks the
power-on frame directly. This directory retains the complete failed metrics,
UART output, result, snapshot, JUnit report, and build/run logs.
