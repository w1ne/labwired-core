# Onboarding Smoke: stm32f103

- status: `fail`
- run_attempted: `True`
- booted: `True`
- smoke_passed: `False`
- elapsed_seconds: `656`
- elapsed_ms: `655244`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `run_smoke`
- first_error_signature: `2026-11-19T01:51:14.926670Z ERROR labwired_cli::execute: Assertion failed: UartContains(UartContainsAssertion { uart_contains: "USART2 RX GATE re_off=1 ue_off=1 data=00005253 released=1 ok" }) (captured len=4769)`
- failure_hint: `Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output.`
