# Retained failed F746 attempt

The firmware built and executed 8,000,000 instructions. The new PendSV check
passed (`pendsv m7 clear=1 repend=1 systick=1 priority=1 ok`), but the existing
SPI1/2/3 assertions sampled NVIC pending after the RX FIFO had drained, so all
three level-sensitive RXNE lines had deasserted. The ordered UART and summary
assertions consequently failed. This attempt is retained to show the original
smoke mismatch; the final fixture samples pending while RXNE is active.
