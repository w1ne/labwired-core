# Onboarding Smoke: stm32h743

- status: `fail`
- run_attempted: `True`
- booted: `True`
- smoke_passed: `False`
- elapsed_seconds: `3`
- elapsed_ms: `2336`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `run_smoke`
- first_error_signature: `2026-11-10T23:24:45.609573Z ERROR labwired_cli::execute: Assertion failed: UartContains(UartContainsAssertion { uart_contains: "cortex-m7 fp lazy unused=1 lspact=00000001,00000001->00000000 untouched=1 regs=1" }) (captured len=988)`
- failure_hint: `Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output.`
