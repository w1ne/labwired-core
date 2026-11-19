# egis_et171 negative control: uart0 removed (2026-11-08)

Same firmware (`--features et171-uart`) and smoke script, with `uart0` removed
from the chip (the `egis_et171.yaml` copy here, loaded through a one-line
system manifest `chip: egis_et171.yaml` with `labwired test --system`).
FAIL 1/6: the first uart0 access (the LCR write at 0xF020002C) is a memory
access violation and the run stops after 13 instructions, so no UART bytes
appear, the LSR/divisor and pass words at 0x1004..0x100C are never written,
and the run does not reach max_steps. Only the MUL word at 0x1000 (written
before any peripheral access) holds. The UART and register checks therefore
depend on the modelled uart0.
