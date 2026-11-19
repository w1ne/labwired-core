# ezr32wg330 — negative control: the old 1-peripheral descriptor (2026-11-09)

Run: firmware-ezr32-330-smoke (this round, linked at 0x00000000) against
the descriptor at 8a91eaa22 (`chip.yaml`: flash at 0x08000000, UART0 only).
The system and smoke script are this round's.

Result: FAIL 1/5 (only `expected_stop_reason`). The old descriptor maps
flash at 0x08000000, but Renode ezr32lg330.repl and the EZR32 memory map put
flash at 0x00000000, so the image is not where the core fetches its vector
table: 0 UART bytes captured, summary word 0x0, no GPIO edges. The old
descriptor only "passed" because its firmware was also linked at
0x08000000.
