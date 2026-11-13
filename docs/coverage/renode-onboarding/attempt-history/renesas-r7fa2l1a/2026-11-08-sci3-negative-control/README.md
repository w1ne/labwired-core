# renesas-r7fa2l1a negative control: SCI3 removed (2026-11-08)

Same firmware and smoke script, with `sci3` removed from the chip (the
`renesas-r7fa2l1a.yaml` copy here). FAIL 2/6: no UART bytes, no P503 edges,
and the SCR/PODR readback words are never written. Only the MULS result and
the max_steps stop reason still hold. The first SCI3 access (the SCR write at
0x40070062, PC 0x866) is unmapped and escalates to a HardFault
(CFSR 0x8200 PRECISERR, BFAR 0x40070062), so the UART/GPIO checks depend on
the modelled SCI.
