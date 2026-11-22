# Negative control: SAM4S16C UART0/UART1 without IRQ lines (2026-11-20)

Firmware: `firmware-sam4s16c-smoke` with the UART0/UART1 TXRDY IRQ check
(bits 14/15). Descriptor: `configs/chips/onboarding/sam4s16c.yaml` from
`origin/main` before this change (no `irq` on `uart0`/`uart1`).

Result: `uart0 txrdy imr=2 ispr8=00 rel=1 FAIL`,
`uart1 txrdy imr=2 ispr8=00 rel=1 FAIL`; summary 0x5A403FFF, smoke 1/3.
With IRQs 8/9 declared the same firmware reads `ispr8=01` / `ispr8=02` and
passes 3/3 (`metrics/sam4s16c`).
