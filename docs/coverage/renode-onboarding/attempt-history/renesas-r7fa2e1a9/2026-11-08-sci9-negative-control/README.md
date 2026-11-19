# renesas-r7fa2e1a9 negative control: SCI9 removed (2026-11-08)

Same firmware and smoke script, with `sci9` removed from the chip (the
`renesas-r7fa2e1a9.yaml` copy here). FAIL 2/6: no UART bytes, no P915 edges,
and the SCR/PODR readback words are never written. Only the MULS result and
the max_steps stop reason still hold. The first SCI9 access (the SCR write at
0x40070122, PC 0x868) is unmapped and escalates to a HardFault
(CFSR 0x8200 PRECISERR, BFAR 0x40070122), so the UART/GPIO checks depend on
the modelled SCI.
