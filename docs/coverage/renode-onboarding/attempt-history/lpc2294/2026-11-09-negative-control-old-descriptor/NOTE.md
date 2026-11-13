# lpc2294 — negative control: this round's firmware on the old descriptor (2026-11-09)

Run: `tests/fixtures/lpc2294-arm7-uart-smoke.elf` (this round, linked at 0x0)
against the descriptor at f358648ca (`chip.yaml`: flash 0x08000000, RAM
0x20000000, UART0 offset-zero TX sink only).

Result: FAIL 0/3 at step 0: "Trying to execute code outside RAM or ROM at
0x00000000". The old descriptor maps no memory at the LPC2294 reset vector;
its smoke passed only because that firmware was linked at 0x08000000 too.
