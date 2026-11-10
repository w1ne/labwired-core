# STM32F072B-DISCO RX gate attempt — 2026-11-10

- Exact imported system booted 4,000,000 instructions and emitted the expected USART2 RX gate result: both disabled-enable checks stayed clear, `R`,`S` arrived in order, and RXNE cleared after the drain.
- The smoke failed only because its ordered UART assertion still expected the old summary value `0x0DC0FFFF`; the firmware reported the new RX-gate summary value `0x0DC1FFFF`.
- Evidence is preserved here before retrying the target-only pipeline.
