# eos-s3 — negative control: SPT at half the core clock (2026-11-16)

Run: `firmware-eos-s3-smoke` (this round, SHA-256 af5bed156746e9b8ad97554e81eda2eab3afc6000acae6946cc33d2d3718bbab) with `examples/renode-onboarding/eos-s3-smoke.yaml` against this round's descriptor with `spt.config.clock_hz: 30720000` (`chip.yaml`). Result: FAIL 5/7 checks. UART `SPT ms=00000007 cycles=00007800`: one SPT millisecond took 30,720 SysTick cycles instead of 61,440, so check 11 fails (`EOS S3 checks PPPPPPPPPPPF DONE`). The check measures the rate, not only that the counter moves.
