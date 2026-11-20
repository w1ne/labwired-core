# renesas-da14592 — negative control: old-descriptor (2026-11-09)

Run: this round's firmware and smoke against the descriptor before this round (UART0 only, no size, no mirrors, no GPIO). Result: FAIL 1/6 checks. HardFault from a precise BusFault at 0x00800100 (SRAM low view, check 7) at PC 0x000009A2. The UART1 and GPIO accesses before it did not fault: UART0's default-size window covered 0x50020100 and 0x50020600.
