# eos-s3 — negative control: HEAD descriptor (2026-11-15)

Run: `firmware-eos-s3-smoke` (this round, SHA-256 1c99bead4e5d3ea1b271b01b29bac1388982e87ec323012fb6035f43bb02be4d) with `examples/renode-onboarding/eos-s3-smoke.yaml` against the descriptor at cf4d649ea (no `intr_ctrl` at 0x40004800). Result: FAIL 1/5 checks. HardFault from a precise BusFault at 0x40004840 (SOFTWARE_INTR_1, check 6) at PC 0x646.
