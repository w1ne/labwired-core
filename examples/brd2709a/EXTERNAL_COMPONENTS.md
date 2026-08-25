# External Components (BRD2709A / xG26-EK2709A)

No required external simulated components for the minimal deterministic smoke test.

The onboarding path uses on-chip peripherals only:

1. USART1 (VCOM console, TX @ 0x400A4038)
2. SysTick (declared; not exercised by the smoke firmware)
3. NVIC (declared; not exercised by the smoke firmware)

On-board hardware not modelled (documented for completeness, not needed at L1):

- mikroBUS socket and Qwiic connector (expansion headers; nothing wired by default)
- On-board J-Link OB debugger (host-side; the sim replaces it)
- Board controller (VCOM routing is implicit in the UART model's TX sink)
