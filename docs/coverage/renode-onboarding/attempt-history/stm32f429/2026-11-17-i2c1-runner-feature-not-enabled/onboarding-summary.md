# Onboarding Smoke: stm32f429

- status: `fail`
- run_attempted: `True`
- booted: `True`
- smoke_passed: `False`
- elapsed_seconds: `115`
- elapsed_ms: `114263`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `run_smoke`
- first_error_signature: `2026-11-17T12:41:58.994374Z ERROR labwired_cli::execute: Assertion failed: UartContains(UartContainsAssertion { uart_contains: "i2c1 bme280_id=00000060 gated=1 clock=1 ok" }) (captured len=371)`
- failure_hint: `Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output.`
