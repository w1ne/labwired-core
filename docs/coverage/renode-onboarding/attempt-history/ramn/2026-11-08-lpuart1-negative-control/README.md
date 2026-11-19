# ramn negative control: LPUART1 removed (2026-11-08)

Same firmware and smoke script, with `lpuart1` removed from the chip (the
`ramn.yaml` copy here, loaded through a one-line system manifest
`chip: ramn.yaml` with `labwired test --system`). FAIL 2/6: no UART bytes, no
PB6 edges, and the result word at 0x20001004 is never written. Only the MUL
word (written before any peripheral access) and the max_steps stop reason
still hold. The first LPUART1 access (the BRR write at 0x4000800C, PC
0x080008C0) is unmapped and escalates to a HardFault (CFSR 0x8200 PRECISERR,
BFAR 0x4000800C). The RCC and GPIO accesses before it succeed. The UART and
GPIO-heartbeat checks therefore depend on the modelled LPUART1.
