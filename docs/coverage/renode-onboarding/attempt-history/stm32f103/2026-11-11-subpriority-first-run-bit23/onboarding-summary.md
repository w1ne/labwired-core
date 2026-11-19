# Onboarding Smoke: stm32f103

- status: `fail`
- run_attempted: `True`
- booted: `True`
- smoke_passed: `False`
- elapsed_seconds: `5`
- elapsed_ms: `4888`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `run_smoke`
- first_error_signature: `2026-11-11T16:02:04.802688Z ERROR labwired_cli::execute: Assertion failed: UartContains(UartContainsAssertion { uart_contains: "NVIC M3 SUBPRIORITY ORDER=55,54 SAME_GROUP_PREEMPT=0 PENDING=1 ok" }) (captured len=3241)`
- failure_hint: `Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output.`
