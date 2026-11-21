# Onboarding Smoke: stm32f412

- status: `fail`
- run_attempted: `True`
- booted: `True`
- smoke_passed: `False`
- elapsed_seconds: `118`
- elapsed_ms: `117372`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `run_smoke`
- first_error_signature: `2026-11-17T05:13:26.088451Z ERROR labwired_cli::execute: Assertion failed: UartContains(UartContainsAssertion { uart_contains: "adc1 in1=000003FF gated=1 clock=1 eoc=1 ok" }) (captured len=175)`
- failure_hint: `Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output.`
