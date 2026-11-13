# sam_e70 — negative control: the old 4-peripheral descriptor (2026-11-09)

Run: firmware-sam-e70-renode-smoke (this round) against the descriptor at
a8e449c9e (`chip.yaml`: PMC, PIOC, USART1 only). The system and smoke
script are this round's.

Result: FAIL. The banner prints, then the first RSTC_SR read (0x400E1804)
is a precise BusFault escalated to HardFault: RSTC is not mapped in the old
descriptor. 0 of 14 checks; the 7 secondary UART lines are absent
(captured 21 bytes). The smoke script therefore depends on the new
descriptor, not on the firmware alone.
