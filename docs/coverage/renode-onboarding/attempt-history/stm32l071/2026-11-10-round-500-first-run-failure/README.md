# STM32L071 Round 500 first run

The target booted and all five new USART/LPUART RX checks printed passing UE/RE,
RXNE data-order, and RXNEIE level results. The run's assertions failed because
the expected UART strings omitted the `0x` prefix emitted by the firmware's
hex formatter. The existing SPI1 and SPI2 IRQ checks also reported `pend=0` in
this attempt. The exact UART log, JUnit, result, snapshot, build logs, and
metrics are preserved here. This attempt is not counted as a passing smoke.
