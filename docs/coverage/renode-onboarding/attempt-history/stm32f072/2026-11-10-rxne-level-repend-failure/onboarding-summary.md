# Onboarding Smoke: stm32f072

- status: `fail`
- run_attempted: `True`
- booted: `True`
- smoke_passed: `False`
- elapsed_seconds: `3`
- elapsed_ms: `2334`
- threshold_seconds: `3600`
- threshold_met: `True`
- failure_stage: `run_smoke`
- first_error_signature: `2026-11-10T15:30:13.386411Z ERROR labwired_cli::execute: Assertion failed: UartOrdered(UartOrderedAssertion { uart_ordered: ["rcc reset=1 gate=1 ok", "usart irq u2=1 u3=1 u4=1 ok", "gpio pa=1 reset=1 pf=1 de=1 ok", "exti swier masked=1 rout`
- failure_hint: `Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output.`
