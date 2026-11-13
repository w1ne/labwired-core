# max32652 — negative control: old-descriptor (2026-11-09)

Run: `firmware-max32652-smoke` (this round) with `examples/renode-onboarding/max32652-smoke.yaml` against the descriptor at 6002b5abb (chip.yaml: UART0 only). Result: FAIL 1/7 checks. HardFault from a precise BusFault at 0x40043008 (UART1 STAT, check 1) at PC 0x10000476; no UART output, summary word 0.
