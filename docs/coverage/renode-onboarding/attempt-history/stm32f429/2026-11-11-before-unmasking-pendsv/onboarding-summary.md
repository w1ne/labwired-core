# Onboarding Smoke: stm32f429

- status: `fail`
- run_attempted: `True`
- booted: `True`
- smoke_passed: `False`
- elapsed_seconds: `9`
- elapsed_ms: `8575`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `run_smoke`
- first_error_signature: `2026-11-11T21:41:28.604329Z ERROR labwired_cli::execute: Assertion failed: UartContains(UartContainsAssertion { uart_contains: "systick pendsv psp tailchain frame=1 r0=1 pending=1 ipsr=0000000E ok" }) (captured len=131)`
- failure_hint: `Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output.`
