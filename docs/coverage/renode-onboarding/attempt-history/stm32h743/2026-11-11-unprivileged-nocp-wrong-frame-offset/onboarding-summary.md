# Onboarding Smoke: stm32h743

- status: `fail`
- run_attempted: `True`
- booted: `True`
- smoke_passed: `False`
- elapsed_seconds: `3`
- elapsed_ms: `2575`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `run_smoke`
- first_error_signature: `2026-11-11T00:53:10.731593Z ERROR labwired_cli::execute: Assertion failed: UartContains(UartContainsAssertion { uart_contains: "cortex-m7 cpacr nocp fault=1 ipsr=00000006 cfsr=00080000 extended=1" }) (captured len=975)`
- failure_hint: `Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output.`
