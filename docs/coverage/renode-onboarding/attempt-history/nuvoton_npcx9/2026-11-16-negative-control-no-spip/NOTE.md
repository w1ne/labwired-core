# nuvoton_npcx9 — negative control: no SPIP (2026-11-16)

Run: `firmware-nuvoton-npcx9-smoke` (this round, SHA-256 31a428efb777bf6e8140594cdef81c6514316afcc82a3bd0f221bad4eb4d8314) and this round's CLI with `examples/renode-onboarding/nuvoton_npcx9-smoke.yaml` against the previous descriptor (f55d31146, `chip.yaml`), which has no `spip` entry. Result: FAIL 12/16 checks. HardFault from a precise BusFault at 0x400D2002 (SPIP CTL1 read, first access of check 20); checks 1..19 before it print `ok`.
