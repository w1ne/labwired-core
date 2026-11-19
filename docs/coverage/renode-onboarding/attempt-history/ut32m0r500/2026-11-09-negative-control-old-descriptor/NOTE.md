# ut32m0r500 — negative control: old-descriptor (2026-11-09)

Run: this round's firmware and smoke against the descriptor before this round (UART0 only, no UART1, no GPIO). Result: FAIL 2/7 assertions. HardFault on a data access at 0x4002_0010 (GPIO0 OUTENSET, check 1) at PC 0x0100_014E. Only the UART0 banner printed.
