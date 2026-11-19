# nuvoton_npcx9 — negative control: no HFCG / LFCG windows (2026-11-16)

Run: `firmware-nuvoton-npcx9-smoke` (this round, SHA-256 54805aef9850fbdc3a52b2a7c94951ae8f706d6a34b236a1b4632e857dc05f3b) and this round's CLI with `examples/renode-onboarding/nuvoton_npcx9-smoke.yaml` against the previous descriptor (f84508b84, `chip.yaml`), which has no `hfcg` / `lfcg` entries. Result: FAIL 10/13 checks. HardFault from a precise BusFault at 0x400B5006 (HFCGN write, first access of check 18); checks 1..17 before it print `ok`.
