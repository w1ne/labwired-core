# andes_ae350_n25 negative control: uart1 removed (2026-11-08)

Same firmware (`--features ae350-uart`) and smoke script, with `uart1` removed
from the chip (the `andes_ae350_n25.yaml` copy here, loaded through a one-line
system manifest `chip: andes_ae350_n25.yaml` with `labwired test --system`).
FAIL 1/6: the first uart1 access (the LCR write at 0xF030002C) is a memory
access violation and the run stops, so no UART bytes appear, the LSR/divisor
and pass words at 0x1004..0x100C are never written, and the run does not reach
max_steps. Only the MUL word at 0x1000 (written before any peripheral access)
holds. The UART and register checks therefore depend on the modelled uart1.

The round's first real run also failed, before any firmware executed: the chip
declared `size: 0x20`, which the descriptor parser rejects ("Invalid size
format: invalid multiple"); it was changed to `size: 32B`. That run's logs were
overwritten by the passing run.
