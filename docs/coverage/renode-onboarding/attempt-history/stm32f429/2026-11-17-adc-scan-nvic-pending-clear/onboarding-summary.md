# Onboarding Smoke: stm32f429

- status: `fail`
- run_attempted: `True`
- booted: `True`
- smoke_passed: `False`
- elapsed_seconds: `125`
- elapsed_ms: `124189`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `run_smoke`
- first_error_signature: `2026-11-17T19:59:50.201470Z ERROR labwired_cli::execute: Assertion failed: UartContains(UartContainsAssertion { uart_contains: "adc1 scan=000003FF,000007FF irq=1,1 end=1 ok" }) (captured len=598)`
- failure_hint: `Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output.`
