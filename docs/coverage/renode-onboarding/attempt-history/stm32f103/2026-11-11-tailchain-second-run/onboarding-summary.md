# Onboarding Smoke: stm32f103

- status: `fail`
- run_attempted: `True`
- booted: `True`
- smoke_passed: `False`
- elapsed_seconds: `3`
- elapsed_ms: `2073`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `run_smoke`
- first_error_signature: `2026-11-11T17:54:36.401429Z ERROR labwired_cli::execute: Assertion failed: UartContains(UartContainsAssertion { uart_contains: "NVIC M3 NESTED IRQ54=1 IRQ55=1 ACTIVE=54,55 RESUMED=54 ok" }) (captured len=3821)`
- failure_hint: `Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output.`
