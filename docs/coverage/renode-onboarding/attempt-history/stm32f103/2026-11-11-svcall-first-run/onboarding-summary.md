# Onboarding Smoke: stm32f103

- status: `fail`
- run_attempted: `True`
- booted: `True`
- smoke_passed: `False`
- elapsed_seconds: `5`
- elapsed_ms: `4837`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `run_smoke`
- first_error_signature: `2026-11-11T16:22:42.299881Z ERROR labwired_cli::execute: Assertion failed: UartContains(UartContainsAssertion { uart_contains: "SVC M3 VECTOR=11 IMM=42 EXC_RETURN=FFFFFFF9 STACKED_PC=RESUME RETURN=1 ok" }) (captured len=3394)`
- failure_hint: `Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output.`
