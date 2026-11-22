# Onboarding Smoke: stm32f429

- status: `fail`
- run_attempted: `True`
- booted: `True`
- smoke_passed: `False`
- elapsed_seconds: `3`
- elapsed_ms: `2658`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `run_smoke`
- first_error_signature: `2026-11-17T20:35:42.961887Z ERROR labwired_cli::execute: Assertion failed: UartContains(UartContainsAssertion { uart_contains: "adc1 scan_cont_groups=000007FF,000007FF,000007FF groups=3 irq=1 ovr=0 stop=1 ok" }) (captured len=698)`
- failure_hint: `Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output.`
