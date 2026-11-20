# nuvoton_npcx9 — negative control: no TWD window (2026-11-16)

Run: `firmware-nuvoton-npcx9-smoke` (this round, SHA-256 197d3e2fa7f96128e5f97081458f741a4d18daa1ccef287099a213934edcb582) and this round's CLI with `examples/renode-onboarding/nuvoton_npcx9-smoke.yaml` against the previous descriptor (77eed9111, `chip.yaml`), which has no `twd` entry. Result: FAIL 0/12 checks. HardFault from a precise BusFault at 0x400D8006 (TWD T0CSR read at entry, the second-boot test); no UART output.
