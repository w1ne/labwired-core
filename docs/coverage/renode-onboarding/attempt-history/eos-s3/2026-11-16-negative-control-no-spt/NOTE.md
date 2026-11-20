# eos-s3 — negative control: no SPT window (2026-11-16)

Run: `firmware-eos-s3-smoke` (this round, SHA-256 af5bed156746e9b8ad97554e81eda2eab3afc6000acae6946cc33d2d3718bbab) with `examples/renode-onboarding/eos-s3-smoke.yaml` against this round's descriptor without the `spt` entry (`chip.yaml`). Result: FAIL 1/7 checks. HardFault from a precise BusFault at 0x40005C00 (SPT_CFG, check 11), no UART output.
