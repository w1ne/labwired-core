# Onboarding Smoke: stm32h743

- status: `fail`
- run_attempted: `True`
- booted: `True`
- smoke_passed: `False`
- elapsed_seconds: `4`
- elapsed_ms: `3048`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `run_smoke`
- first_error_signature: `2026-11-11T02:19:45.235650Z ERROR labwired_cli::execute: Assertion failed: UartContains(UartContainsAssertion { uart_contains: "cortex-m7 cpacr handler fp=1 ipsr=0000000B nocp=00000000 result=000000C3" }) (captured len=1549)`
- failure_hint: `Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output.`
