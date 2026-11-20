# s32k118 negative control: LPUART0 removed (2026-11-08)

Same firmware and smoke script, with `lpuart0` removed from the chip (the
`s32k118.yaml` copy here). FAIL 2/6: no UART bytes, no PTD0 edges, and the
pass word at 0x20000004 is never written. Only the MULS result and the
max_steps stop reason still hold. The first LPUART0 access (the BAUD write,
r3 = 0x4006A010) is unmapped and the core ends in exception context
(LR = EXC_RETURN 0xFFFFFFF9), so the UART/GPIO checks depend on the modelled
LPUART.
