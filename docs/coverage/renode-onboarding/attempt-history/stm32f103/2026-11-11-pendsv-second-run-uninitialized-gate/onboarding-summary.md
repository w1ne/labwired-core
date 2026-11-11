# Onboarding Smoke: stm32f103

- status: `fail`
- run_attempted: `True`
- booted: `True`
- smoke_passed: `False`
- elapsed_seconds: `1`
- elapsed_ms: `855`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `run_smoke`
- first_error_signature: `2026-11-11T17:04:39.068857Z ERROR labwired_cli::execute: Assertion failed: UartContains(UartContainsAssertion { uart_contains: "SYSTICK M3 PERIODIC=1 REPEATED=1 VECTOR=15 IPSR=15 RETURN=1 ok" }) (captured len=154)`
- failure_hint: `Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output.`
