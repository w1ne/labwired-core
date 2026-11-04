# EFM32PG1 attempt 01: incorrect USART0 base

The first smoke fixture reused the EFM32PG12 USART0 address (`0x4000c000`).
The imported EFM32PG1 descriptor places USART0 at `0x40010000`, so the write
faulted as unmapped MMIO before any UART bytes were emitted.

This run executed 100,000 instructions, but the UART assertion failed and the
result reported a precise BusFault/HardFault at `0x4000c00c`. It is preserved
here as a failed attempt and is not counted in the active scoreboard. The
fixture was corrected to use the PG1 descriptor's USART0 base before the
measured inventory refresh.
