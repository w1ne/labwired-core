# eos-s3 — negative control: old-descriptor (2026-11-09)

Run: `firmware-eos-s3-smoke` (this round) with `examples/renode-onboarding/eos-s3-smoke.yaml` against the descriptor at 6a85a030e (chip.yaml: separate 512 KiB stores at 0x0 and 0x20000000, PL011 only). Result: FAIL 1/5 checks. HardFault from a precise BusFault at 0x4000F000 (SDMA SRAM, check 3) at PC 0x52A; no UART output, summary word 0.
