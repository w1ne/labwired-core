# nuvoton_npcx9 — negative control: no MTC window (2026-11-16)

Run: `firmware-nuvoton-npcx9-smoke` (this round, SHA-256 85e272a65f6bcc187b74b65d78714b7a432538c3a21ad70253e81ecd2b9b46a4) and this round's CLI with `examples/renode-onboarding/nuvoton_npcx9-smoke.yaml` against the previous descriptor (d62cc5afa, `chip.yaml`), which has no `mtc` entry. Result: FAIL 9/10 checks. HardFault from a precise BusFault at 0x400B7000 (MTC TTC write, check 16); no `mtc` line.
