# Onboarding Smoke: stm32l071

- status: `fail`
- run_attempted: `True`
- booted: `True`
- smoke_passed: `False`
- elapsed_seconds: `10`
- elapsed_ms: `9927`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `run_smoke`
- first_error_signature: `2026-11-10T15:09:26.564063Z ERROR labwired_cli::execute: Assertion failed: UartOrdered(UartOrderedAssertion { uart_ordered: ["rcc reset=1 gate=1 ok", "usart irq u1=1 u2=1 u4=1 u5=1 lp1=1 ok", "usart rx u1 re_off=1 ue_off=1 data=00005253 rxn`
- failure_hint: `Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output.`
