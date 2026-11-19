# eos-s3 — negative control: split-stores (2026-11-09)

Run: `firmware-eos-s3-smoke` (this round) with `examples/renode-onboarding/eos-s3-smoke.yaml` against this round's descriptor with the memory map reverted to two separate 512 KiB stores (flash 0x0, RAM 0x20000000, no m4mem_low mirror). Result: FAIL 3/5 checks. UART `EOS S3 checks FFFPPP DONE`; summary 0xE0530038. Checks 0-2 (code read through 0x20000000, .data read through 0x0, store through 0x0) fail: the views do not share bytes.
