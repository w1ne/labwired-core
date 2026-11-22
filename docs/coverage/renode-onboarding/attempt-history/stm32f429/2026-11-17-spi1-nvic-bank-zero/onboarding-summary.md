# Onboarding Smoke: stm32f429

- status: `fail`
- run_attempted: `True`
- booted: `True`
- smoke_passed: `False`
- elapsed_seconds: `218`
- elapsed_ms: `217946`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `run_smoke`
- first_error_signature: `2026-11-17T12:00:07.954781Z ERROR labwired_cli::execute: Assertion failed: UartContains(UartContainsAssertion { uart_contains: "spi1 max31855=01901600 gated=1 clock=1 txeirq=1 rearm=0 rxneirq=1 released=1 ok" }) (captured len=373)`
- failure_hint: `Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output.`
