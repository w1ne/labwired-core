# Onboarding Smoke: stm32f103

- status: `fail`
- run_attempted: `True`
- booted: `True`
- smoke_passed: `False`
- elapsed_seconds: `87`
- elapsed_ms: `86457`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `run_smoke`
- first_error_signature: `2026-11-11T17:53:50.741545Z ERROR labwired_cli::execute: Assertion failed: UartContains(UartContainsAssertion { uart_contains: "NVIC M3 TAILCHAIN IRQ54=1 IRQ55=1 FRAME=REUSED R0=LIVE ok" }) (captured len=3725)`
- failure_hint: `Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output.`
