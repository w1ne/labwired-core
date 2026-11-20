# Failed STM32F103 USART2 RX DMA attempt

The firmware received all four injected UART bytes: the existing CPU RX gate
read `RS` and the new DMA1 channel 6 check read `DM` correctly. The attempt
failed two pre-existing/new gate assertions for setup reasons. The RX gate
still expected the entire RX FIFO to be empty after consuming only `RS`, even
though `DM` was intentionally queued for DMA. The profile also enabled
per-channel DMA IRQ lines; the resulting live-line behavior changed the older
channel 1 `NVIC_CLEAR` assertion. The DMA data path itself passed, including
DMAR-off blocking, CNDTR zero, TCIF6 and IRQ16.

The retry keeps the `DM` queue visible as an RX gate assertion and leaves
channel 6 TCIE disabled, so this bounded imported profile does not claim a
channel 6 interrupt route.
