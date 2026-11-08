# s32k118 attempt: LPUART0 smoke without `cpu_hz` (2026-11-08)

FAIL 3/6. The chip declared no `cpu_hz`, so the i.MX RT LPUART timebase used
its 600 MHz default: one 8N1 frame at OSR=15/SBR=26 from the 48 MHz LPUART
clock took 10 * 16 * 26 * 600e6 / 48e6 = 52,000 CPU cycles. 2,000,000 steps
then fit 38 characters, which is exactly the captured UART length; the CPU was
still polling STAT.TDRE (r2 = 0x4006A014). Not a model fault. Fixed by
declaring the S32K118's 48 MHz core clock (`cpu_hz: 48000000`).
