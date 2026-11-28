# zolertia-firefly — negative control: old-descriptor (2026-11-09)

Run: `firmware-zolertia-firefly-smoke` (this round) with `examples/renode-onboarding/zolertia-firefly-smoke.yaml` against the descriptor at 3ed42265c (chip.yaml: CPU, memory, UART0 only). Result: FAIL 1/7 checks. HardFault from a precise BusFault at 0x400D2028 (SYS_CTRL RCGCUART, the first store) at PC 0x0020045C; no UART output, summary word 0.
