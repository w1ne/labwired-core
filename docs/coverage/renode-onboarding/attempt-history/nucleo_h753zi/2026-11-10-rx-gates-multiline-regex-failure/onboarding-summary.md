# Onboarding Smoke: nucleo_h753zi

- status: `fail`
- run_attempted: `True`
- booted: `True`
- smoke_passed: `False`
- elapsed_seconds: `3`
- elapsed_ms: `2121`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `run_smoke`
- first_error_signature: `2026-11-10T13:00:20.269262Z ERROR labwired_cli::execute: Assertion failed: UartRegex(UartRegexAssertion { uart_regex: "^STM32H7 register smoke\nusart3 rx_gate re_off=1 ue_off=1 ok\nusart3 rx=00000052,00000053 rxne_irq=isolated,asserted,reas`
- failure_hint: `Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output.`
