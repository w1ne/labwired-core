# eos-s3 — negative control: no-gpio (2026-11-09)

Run: `firmware-eos-s3-smoke` (this round) with `examples/renode-onboarding/eos-s3-smoke.yaml` against this round's descriptor without the `gpio` MISC IO port. Result: FAIL 1/5 checks. HardFault from a precise BusFault at 0x40005104 (IO_OUTPUT, check 4) at PC 0x576.
