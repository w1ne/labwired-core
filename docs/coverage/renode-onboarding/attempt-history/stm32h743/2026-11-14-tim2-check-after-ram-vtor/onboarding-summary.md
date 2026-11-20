# Onboarding Smoke: stm32h743

- status: `fail`
- run_attempted: `True`
- booted: `True`
- smoke_passed: `False`
- elapsed_seconds: `945`
- elapsed_ms: `944151`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `run_smoke`
- first_error_signature: `2026-11-14T13:55:46.290600Z ERROR labwired_cli::execute: Assertion failed: UartContains(UartContainsAssertion { uart_contains: "tim2 gated=1 arr_reset=1 w32=1 ug=1 uif_clr=1 run=1 masked=1 irq=3 stray=0 stop=1 opm=1 ok" }) (captured len=489`
- failure_hint: `Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output.`
