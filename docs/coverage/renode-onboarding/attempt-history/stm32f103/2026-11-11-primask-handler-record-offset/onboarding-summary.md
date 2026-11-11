# Onboarding Smoke: stm32f103

- status: `fail`
- run_attempted: `True`
- booted: `True`
- smoke_passed: `False`
- elapsed_seconds: `5`
- elapsed_ms: `4418`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `run_smoke`
- first_error_signature: `2026-11-11T15:38:27.531936Z ERROR labwired_cli::execute: Assertion failed: UartContains(UartContainsAssertion { uart_contains: "PRIMASK M3 MASKED=1 PENDING=1 RELEASED=1 VECTOR=54 ok" }) (captured len=3264)`
- failure_hint: `Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output.`
