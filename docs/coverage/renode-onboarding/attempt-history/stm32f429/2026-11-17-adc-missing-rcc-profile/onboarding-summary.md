# Onboarding Smoke: stm32f429

- status: `fail`
- run_attempted: `True`
- booted: `True`
- smoke_passed: `False`
- elapsed_seconds: `126`
- elapsed_ms: `125321`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `run_smoke`
- first_error_signature: `2026-11-17T04:41:31.909861Z ERROR labwired_cli::execute: Assertion failed: UartContains(UartContainsAssertion { uart_contains: "adc1 in1=000003FF gated=1 clock=1 eoc=1 ok" }) (captured len=208)`
- failure_hint: `Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output.`
