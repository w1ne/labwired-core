# nuvoton_npcx9 — negative control: no memory windows (2026-11-16)

Run: `firmware-nuvoton-npcx9-smoke` (this round, SHA-256 4e7b49d914b058a184c73b0417631dc14cb8f0e3348031b50137a470dd1d80b6) and this round's CLI with `examples/renode-onboarding/nuvoton_npcx9-smoke.yaml` against the previous descriptor (62232dc93, `chip.yaml`), which has no `memory_regions`. Result: FAIL 11/15 checks. HardFault from a precise BusFault at 0x10058000 (code_ram write, first access of check 19); checks 1..18 before it print `ok`.
