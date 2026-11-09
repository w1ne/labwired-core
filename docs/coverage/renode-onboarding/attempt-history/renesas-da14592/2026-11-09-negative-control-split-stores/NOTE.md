# renesas-da14592 — negative control: split-stores (2026-11-09)

Run: this round's firmware and smoke against a descriptor whose SRAM, ROM and eFlash-data low views are separate memories, not `alias_of` mirrors. This run used the default UART window (before the 256 B sizes). Result: FAIL 4/6 checks. UART0 prints `DA14592 checks PPPPPPPFFF DONE`; summary word 0xDA14007F. Checks 7 (SRAM 0x800000 view), 8 (ROM 0x900000 view) and 9 (eFlash data 0xA00000 view) fail. This proves the three mirrors share one store.
