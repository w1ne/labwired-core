# fomu negative control: timer0 removed (2026-11-08)

Same firmware (`--features timer0`) and smoke script, with `timer0` removed
from the chip (the `fomu.yaml` copy here, loaded through a one-line system
manifest `chip: <abs path>/fomu.yaml` with `labwired test --script`).
FAIL 1/5: the first timer0 access (the EN write at 0x60002820) is a memory
access violation and the run stops (the CLI reports 0 steps), so the load
readback, check-bit and pass words are never written and the run does not
reach max_steps. Only the MUL word at 0x10000000 (written before any
peripheral access) holds. The timer checks therefore depend on the modelled
timer0.
